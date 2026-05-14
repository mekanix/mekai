use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing::{error, info, warn};

use mekai::kimi::approval::ApprovalRuntime;
use mekai::kimi::cancel::CancelToken;
use mekai::kimi::cli::{Cli, Commands, McpCommands, PluginCommands};
use mekai::kimi::config::{self, Config};
use mekai::kimi::llm;
use mekai::kimi::metadata::{load_metadata, save_metadata};
use mekai::kimi::session::Session;
use mekai::kimi::skill::discover_skills;
use mekai::kimi::soul::tools::init_shared_resources;
use mekai::kimi::soul::{Agent, KimiSoul};
use mekai::kimi::telemetry::{TelemetryEvent, TelemetrySink};
use mekai::kimi::ui::print::Print;
use mekai::kimi::ui::shell::Shell;
use mekai::kimi::utils::logging::init_logging;
use mekai::kimi::wire::WireHub;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args()
        .enumerate()
        .map(|(i, arg)| {
            if i == 0 {
                arg.strip_suffix(".exe")
                    .map(|s| s.to_string())
                    .unwrap_or(arg)
            } else {
                arg
            }
        })
        .collect();

    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    init_logging(cli.debug || cli.verbose);
    info!("starting mekai");

    if let Err(e) = run(cli).await {
        error!("{e:?}");
        eprintln!("Error: {e}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Commands::Term { ref args }) => {
            info!("term args: {args:?}");
            // Toad TUI: launch the shell with optional prompt args
            let prompt = if args.is_empty() {
                None
            } else {
                Some(args.join(" "))
            };
            let config = load_cli_config(&cli)?;
            let work_dir = cli
                .work_dir
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            let model_name = cli
                .model
                .as_deref()
                .or(config.default_model.as_deref())
                .unwrap_or("kimi");
            let llm: Arc<dyn llm::ChatProvider> =
                std::sync::Arc::from(llm::create_llm(&config, model_name).await?);
            let session = Session::create(&work_dir, None).await?;
            let approval = Arc::new(ApprovalRuntime::new());
            if cli.yolo || config.default_yolo.unwrap_or(false) {
                approval.set_yolo(true).await;
            }
            let mut loop_control = config.loop_control.clone();
            if let Some(v) = cli.max_steps_per_turn {
                loop_control.max_steps_per_turn = v;
            }
            if let Some(v) = cli.max_retries_per_step {
                loop_control.max_retries_per_step = v;
            }
            if let Some(v) = cli.max_ralph_iterations {
                loop_control.max_ralph_iterations = v;
            }
            let mut soul = KimiSoul::new(
                Agent {
                    name: "default".to_string(),
                    system_prompt: build_system_prompt(&work_dir, "default"),
                    tools: vec![],
                },
                tokio::sync::broadcast::channel(16).0,
                approval,
                200000,
                loop_control,
            )
            .with_session(session)
            .with_plan_mode(cli.plan || config.default_plan_mode.unwrap_or(false))
            .with_thinking(
                cli.thinking
                    .unwrap_or(config.default_thinking.unwrap_or(false)),
            );
            if !config.hooks.is_empty() {
                soul = soul.with_hooks(mekai::kimi::hooks::HookEngine::new(config.hooks));
            }
            let theme = config.theme.unwrap_or_default();
            let show_thinking = config.show_thinking_stream.unwrap_or(true);
            let mut shell = Shell::new(theme, show_thinking);
            let _exit_code = shell.run(llm, &mut Some(soul), prompt).await?;
        }
        Some(Commands::Export { output }) => {
            export_diagnostics(output).await?;
        }
        Some(Commands::Info) => {
            show_info().await?;
        }
        Some(Commands::Mcp { command }) => match command {
            Some(McpCommands::List) => list_mcp_servers().await?,
            Some(McpCommands::Add { name, command: cmd }) => {
                let mut servers = load_mcp_servers()?;
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                let command = parts.first().copied().unwrap_or("").to_string();
                let args = parts.iter().skip(1).map(|s| s.to_string()).collect();
                servers.servers.insert(
                    name.clone(),
                    mekai::kimi::mcp::McpServerConfig {
                        command,
                        args,
                        env: HashMap::new(),
                    },
                );
                save_mcp_servers(&servers)?;
                println!("Added MCP server: {name}");
            }
            Some(McpCommands::Remove { name }) => {
                let mut servers = load_mcp_servers()?;
                if servers.servers.remove(&name).is_some() {
                    save_mcp_servers(&servers)?;
                    println!("Removed MCP server: {name}");
                } else {
                    println!("MCP server not found: {name}");
                }
            }
            None => {}
        },
        Some(Commands::Plugin { command }) => {
            let pm = mekai::kimi::plugin::PluginManager::new();
            match command {
                Some(PluginCommands::List) => {
                    for plugin in pm.list_plugins()? {
                        println!("{} - {}", plugin.name, plugin.description);
                    }
                }
                Some(PluginCommands::Install { source }) => {
                    pm.install(&source).await?;
                }
                Some(PluginCommands::Remove { name }) => {
                    pm.remove(&name).await?;
                }
                None => {}
            }
        }
        Some(Commands::Vis) => {
            let mut vis = mekai::kimi::ui::vis::Visualizer::new();
            let _exit_code = vis.run().await?;
        }
        None => {
            run_main_command(cli).await?;
        }
    }

    Ok(())
}

async fn run_main_command(cli: Cli) -> Result<()> {
    let mut config = load_cli_config(&cli)?;
    let work_dir = cli
        .work_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Merge CLI skills directories into config
    for dir in &cli.skills_dir {
        if !config.extra_skill_dirs.contains(dir) {
            config.extra_skill_dirs.push(dir.clone());
        }
    }

    let model_name = cli
        .model
        .as_deref()
        .or(config.default_model.as_deref())
        .unwrap_or("kimi");
    let llm: Arc<dyn llm::ChatProvider> =
        std::sync::Arc::from(llm::create_llm(&config, model_name).await?);

    // Setup telemetry
    let mut telemetry = TelemetrySink::new(config.telemetry.unwrap_or(false));
    telemetry.emit(TelemetryEvent {
        event_type: "session_start".to_string(),
        timestamp: chrono::Utc::now(),
        payload: serde_json::json!({"work_dir": work_dir.to_string_lossy()}),
    });

    let (session, existing_messages) = if let Some(ref id) = cli.resume {
        if id.is_empty() {
            // Picker mode
            let sessions = Session::list(&work_dir).await?;
            if sessions.is_empty() {
                println!("No sessions found for the working directory.");
                return Ok(());
            }
            println!("Recent sessions:");
            for (i, s) in sessions.iter().enumerate().take(10) {
                let time_ago = format_time_ago(s.updated_at);
                println!("  {}. {} ({}) - {}", i + 1, s.title, &s.id[..8], time_ago);
            }
            println!("Use --session <id> to resume a specific session.");
            return Ok(());
        } else {
            match Session::find(&work_dir, id).await? {
                Some(s) => {
                    let messages = s.load_context()?;
                    println!("Resuming session: {} ({})", s.title, &s.id[..8]);
                    (s, messages)
                }
                None => {
                    println!("Session not found, creating new session.");
                    (Session::create(&work_dir, Some(id.clone())).await?, vec![])
                }
            }
        }
    } else if cli.continue_ {
        match Session::continue_(&work_dir).await? {
            Some(s) => {
                let messages = s.load_context()?;
                println!("Continuing session: {} ({})", s.title, &s.id[..8]);
                (s, messages)
            }
            None => {
                println!("No previous session found, creating new session.");
                (Session::create(&work_dir, None).await?, vec![])
            }
        }
    } else {
        (Session::create(&work_dir, None).await?, vec![])
    };

    let mut session = session;

    for dir in &cli.add_dir {
        let dir_str = dir.to_string_lossy().to_string();
        if !session.state.additional_dirs.contains(&dir_str) {
            session.state.additional_dirs.push(dir_str);
        }
    }
    session.save_state()?;

    let max_context = config
        .models
        .get(model_name)
        .and_then(|m| m.max_context_size)
        .unwrap_or(200000);

    // Initialize shared resources for tools
    let bg_manager = mekai::kimi::background::manager::BackgroundTaskManager::new(
        config.background.max_running_tasks,
        std::time::Duration::from_secs(config.background.agent_task_timeout_s),
        config.background.keep_alive_on_exit,
    );
    init_shared_resources(Arc::clone(&llm), bg_manager);
    mekai::kimi::soul::tools::init_services(
        config.services.moonshot_search.clone(),
        config.services.moonshot_fetch.clone(),
    );

    let approval = Arc::new(ApprovalRuntime::new());
    if cli.yolo
        || cli.print
        || cli.quiet
        || session.state.approval_settings.yolo
        || config.default_yolo.unwrap_or(false)
    {
        approval.set_yolo(true).await;
    }
    for (action, approved) in &session.state.approval_settings.per_action {
        approval.set_per_action(action.clone(), *approved).await;
    }

    let wire_hub = Arc::new(WireHub::new(1024));
    let wire_tx = wire_hub.tx.clone();

    // Build system prompt with AGENTS.md and skills
    let mut system_prompt = build_system_prompt(&work_dir, "default");

    // Load AGENTS.md
    if let Some(agents_md) = find_agents_md(&work_dir)
        && let Ok(content) = std::fs::read_to_string(&agents_md)
    {
        system_prompt.push_str("\n\n## Project Context\n\n");
        system_prompt.push_str(&content);
    }

    // Load skills
    if config.merge_all_available_skills.unwrap_or(true) {
        let skills = discover_skills(&config.extra_skill_dirs)?;
        if !skills.is_empty() {
            system_prompt.push_str("\n\n## Skills\n\n");
            for (name, skill) in &skills {
                system_prompt.push_str(&format!(
                    "### {name}\n{content}\n\n",
                    content = skill.content
                ));
            }
        }
    }

    // Load agent spec if provided
    let mut agent_spec_tools: Vec<String> = vec![];
    let mut agent_spec_model: Option<String> = None;
    let mut agent_spec_thinking: Option<bool> = None;
    let mut agent_spec_name: String = "default".to_string();
    if let Some(ref agent_file) = cli.agent_file {
        if let Ok(spec) = mekai::kimi::agentspec::AgentSpec::load(agent_file) {
            system_prompt = spec.system_prompt;
            agent_spec_tools = spec.tools;
            agent_spec_model = spec.model;
            agent_spec_thinking = spec.thinking;
            agent_spec_name = spec.name.clone();
            if !spec.version.is_empty() {
                info!("Agent spec version: {}", spec.version);
            }
            if !spec.description.is_empty() {
                info!("Agent spec description: {}", spec.description);
            }
            if !spec.extra.is_empty() {
                system_prompt.push_str("\n\n## Extra Configuration\n\n");
                system_prompt
                    .push_str(&serde_json::to_string_pretty(&spec.extra).unwrap_or_default());
            }
            info!("Loaded agent spec: {}", spec.name);
        }
    } else if let Some(agent_file) = mekai::kimi::agentspec::find_agent_file(&work_dir)
        && let Ok(spec) = mekai::kimi::agentspec::AgentSpec::load(&agent_file)
    {
        system_prompt = spec.system_prompt;
        agent_spec_tools = spec.tools;
        agent_spec_model = spec.model;
        agent_spec_thinking = spec.thinking;
        agent_spec_name = spec.name.clone();
        if !spec.version.is_empty() {
            info!("Agent spec version: {}", spec.version);
        }
        if !spec.description.is_empty() {
            info!("Agent spec description: {}", spec.description);
        }
        if !spec.extra.is_empty() {
            system_prompt.push_str("\n\n## Extra Configuration\n\n");
            system_prompt.push_str(&serde_json::to_string_pretty(&spec.extra).unwrap_or_default());
        }
        info!("Loaded agent spec from work dir: {}", spec.name);
    }

    let mut tool_defs: Vec<llm::ToolDef> = mekai::kimi::soul::tools::builtin_tools()
        .into_iter()
        .map(|t| llm::ToolDef {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters(),
        })
        .collect();

    let mut mcp_tools: Vec<Arc<dyn mekai::kimi::soul::tools::Tool>> = vec![];

    // Load MCP tools from files
    for mcp_config_file in &cli.mcp_config_file {
        if let Ok(content) = std::fs::read_to_string(mcp_config_file)
            && let Ok(servers) = serde_json::from_str::<mekai::kimi::mcp::McpServers>(&content)
        {
            for (name, server_config) in servers.servers {
                info!("Connecting to MCP server: {name}");
                match mekai::kimi::mcp::McpClient::connect(&server_config).await {
                    Ok(client) => {
                        let client =
                            Arc::new(client.with_timeout(std::time::Duration::from_millis(
                                config.mcp.client.tool_call_timeout_ms,
                            )));
                        match client.list_tools().await {
                            Ok(tools) => {
                                info!("Loaded {} tools from MCP server {name}", tools.len());
                                for def in &tools {
                                    tool_defs.push(def.clone());
                                    mcp_tools.push(Arc::new(mekai::kimi::mcp::McpTool {
                                        client: Arc::clone(&client),
                                        tool_name: def.name.clone(),
                                        tool_description: def.description.clone(),
                                        tool_parameters: def.parameters.clone(),
                                    }));
                                }
                            }
                            Err(e) => warn!("Failed to list MCP tools for {name}: {e}"),
                        }
                    }
                    Err(e) => warn!("Failed to connect to MCP server {name}: {e}"),
                }
            }
        }
    }

    // Load MCP tools from inline JSON strings
    for mcp_json in &cli.mcp_config {
        if let Ok(servers) = serde_json::from_str::<mekai::kimi::mcp::McpServers>(mcp_json) {
            for (name, server_config) in servers.servers {
                info!("Connecting to MCP server: {name}");
                match mekai::kimi::mcp::McpClient::connect(&server_config).await {
                    Ok(client) => {
                        let client =
                            Arc::new(client.with_timeout(std::time::Duration::from_millis(
                                config.mcp.client.tool_call_timeout_ms,
                            )));
                        match client.list_tools().await {
                            Ok(tools) => {
                                info!("Loaded {} tools from MCP server {name}", tools.len());
                                for def in &tools {
                                    tool_defs.push(def.clone());
                                    mcp_tools.push(Arc::new(mekai::kimi::mcp::McpTool {
                                        client: Arc::clone(&client),
                                        tool_name: def.name.clone(),
                                        tool_description: def.description.clone(),
                                        tool_parameters: def.parameters.clone(),
                                    }));
                                }
                            }
                            Err(e) => warn!("Failed to list MCP tools for {name}: {e}"),
                        }
                    }
                    Err(e) => warn!("Failed to connect to MCP server {name}: {e}"),
                }
            }
        }
    }

    let mut plugin_tools: Vec<Arc<dyn mekai::kimi::soul::tools::Tool>> = vec![];

    // Load plugin tools
    let plugin_manager = mekai::kimi::plugin::PluginManager::new();
    for plugin in plugin_manager.list_plugins()? {
        for tool_def in &plugin.spec.tools {
            tool_defs.push(llm::ToolDef {
                name: tool_def.name.clone(),
                description: tool_def.description.clone(),
                parameters: tool_def.parameters.clone(),
            });
            plugin_tools.push(Arc::new(mekai::kimi::plugin::PluginTool {
                name: tool_def.name.clone(),
                description: tool_def.description.clone(),
                parameters: tool_def.parameters.clone(),
                command: tool_def.command.clone(),
                dir: plugin.dir.clone(),
            }));
        }
    }

    if !agent_spec_tools.is_empty() {
        let allowed: HashSet<String> = agent_spec_tools.iter().cloned().collect();
        tool_defs = mekai::kimi::agentspec::resolve_tools(&agent_spec_tools, &tool_defs);
        mcp_tools.retain(|t| allowed.contains(t.name()));
        plugin_tools.retain(|t| allowed.contains(t.name()));
    }

    // Recreate LLM if agent spec requests a different model
    let llm: Arc<dyn llm::ChatProvider> = if let Some(ref spec_model) = agent_spec_model {
        if spec_model != model_name {
            info!("Switching to agent-specified model: {spec_model}");
            std::sync::Arc::from(llm::create_llm(&config, spec_model).await?)
        } else {
            llm
        }
    } else {
        llm
    };

    // Build loop control from config + CLI overrides
    let mut loop_control = config.loop_control.clone();
    if let Some(v) = cli.max_steps_per_turn {
        loop_control.max_steps_per_turn = v;
    }
    if let Some(v) = cli.max_retries_per_step {
        loop_control.max_retries_per_step = v;
    }
    if let Some(v) = cli.max_ralph_iterations {
        loop_control.max_ralph_iterations = v;
    }

    let agent = Agent {
        name: agent_spec_name,
        system_prompt,
        tools: tool_defs,
    };

    let cancel = Arc::new(CancelToken::new());
    let cancel_signal = Arc::clone(&cancel);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel_signal.cancel();
        }
    });

    let plan_mode =
        cli.plan || config.default_plan_mode.unwrap_or(false) || session.state.plan_mode;
    let thinking = cli
        .thinking
        .or(agent_spec_thinking)
        .or(config.default_thinking)
        .unwrap_or(false);

    let mut soul = KimiSoul::new(agent, wire_tx, approval, max_context, loop_control)
        .with_session(session)
        .with_plan_mode(plan_mode)
        .with_thinking(thinking)
        .with_cancel(cancel);

    if !config.hooks.is_empty() {
        soul = soul.with_hooks(mekai::kimi::hooks::HookEngine::new(config.hooks));
    }

    for tool in mcp_tools {
        soul.tools.insert(tool.name().to_string(), tool);
    }
    for tool in plugin_tools {
        soul.tools.insert(tool.name().to_string(), tool);
    }

    if !existing_messages.is_empty() {
        soul = soul.with_context_messages(existing_messages);
    }

    let ui_mode = if cli.quiet || cli.print {
        "print"
    } else if cli.acp {
        "acp"
    } else if cli.wire {
        "wire"
    } else {
        "shell"
    };

    match ui_mode {
        "shell" => {
            let theme = config.theme.unwrap_or_default();
            let show_thinking = config.show_thinking_stream.unwrap_or(true);
            let mut shell = Shell::new(theme, show_thinking);
            let mut soul_opt = Some(soul);
            let _exit_code = shell.run(llm, &mut soul_opt, cli.prompt).await?;
            soul = soul_opt.unwrap();
        }
        "print" => {
            let output_format = if cli.quiet {
                "text".to_string()
            } else {
                cli.output_format.unwrap_or_else(|| "text".to_string())
            };
            let mut print = Print::new(
                cli.input_format.unwrap_or_else(|| "text".to_string()),
                output_format,
                cli.final_message_only || cli.quiet,
            );
            let _exit_code = print.run(llm, &mut soul, cli.prompt).await?;
        }
        "acp" => {
            let acp = mekai::kimi::acp::AcpServer::new();
            acp.run().await?;
        }
        "wire" => {
            let server = mekai::kimi::wire::server::WireServer::new(wire_hub);
            server.serve_stdio().await?;
        }
        _ => unreachable!(),
    }

    if let Some(mut session) = soul.session {
        session.state.approval_settings.yolo = soul.approval.is_yolo().await;
        session.state.approval_settings.per_action = soul.approval.get_per_action().await;
        session.state.plan_mode = soul.plan_mode;
        session.save_state()?;
        if !session.is_empty() {
            let mut meta = load_metadata()?;
            let wdm = meta.get_or_create_work_dir_meta(&work_dir);
            wdm.last_session_id = Some(session.id.clone());
            save_metadata(&meta)?;
        } else {
            session.delete().await?;
        }
    }

    telemetry.emit(TelemetryEvent {
        event_type: "session_end".to_string(),
        timestamp: chrono::Utc::now(),
        payload: serde_json::json!({}),
    });
    telemetry.flush()?;

    Ok(())
}

fn build_system_prompt(work_dir: &std::path::Path, agent_name: &str) -> String {
    let name_line = if agent_name == "default" {
        "You are Mekai, a helpful CLI agent.".to_string()
    } else {
        format!("You are {agent_name}, a helpful CLI agent.")
    };
    format!(
        "{name_line}\n\nCurrent working directory: {}\n\nYou have access to tools. Use them when helpful. Always respond in the same language as the user's query.",
        work_dir.display()
    )
}

fn find_agents_md(work_dir: &std::path::Path) -> Option<PathBuf> {
    let mut dir = Some(work_dir);
    while let Some(d) = dir {
        let candidate = d.join("AGENTS.md");
        if candidate.exists() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

fn format_time_ago(dt: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(dt);
    if duration.num_days() > 0 {
        format!("{}d ago", duration.num_days())
    } else if duration.num_hours() > 0 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_minutes() > 0 {
        format!("{}m ago", duration.num_minutes())
    } else {
        "just now".to_string()
    }
}

fn load_cli_config(cli: &Cli) -> Result<Config> {
    if let Some(ref config_string) = cli.config {
        Ok(config::load_config_from_string(config_string)?)
    } else if let Some(ref config_file) = cli.config_file {
        Ok(config::load_config_from_file(config_file)?)
    } else {
        Ok(config::load_config().unwrap_or_default())
    }
}

async fn export_diagnostics(output: Option<PathBuf>) -> Result<()> {
    let dest = output.unwrap_or_else(|| PathBuf::from("mekai-diagnostics.json"));
    let sessions_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mekai")
        .join("sessions");
    let mut sessions = vec![];
    if sessions_dir.exists() {
        for entry in std::fs::read_dir(&sessions_dir)? {
            let entry = entry?;
            let session_json = entry.path().join("session.json");
            if session_json.exists()
                && let Ok(content) = std::fs::read_to_string(session_json)
                && let Ok(session) = serde_json::from_str::<serde_json::Value>(&content)
            {
                sessions.push(session);
            }
        }
    }
    let diag = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "sessions": sessions,
    });
    std::fs::write(&dest, serde_json::to_string_pretty(&diag)?)?;
    println!("Diagnostics exported to {}", dest.display());
    Ok(())
}

async fn show_info() -> Result<()> {
    println!("Mekai {}", env!("CARGO_PKG_VERSION"));
    println!("Platform: {}", std::env::consts::OS);
    println!("Arch: {}", std::env::consts::ARCH);
    println!(
        "Config dir: {}",
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mekai")
            .display()
    );
    println!(
        "Data dir: {}",
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mekai")
            .display()
    );
    Ok(())
}

fn mcp_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mekai")
        .join("mcp.json")
}

fn load_mcp_servers() -> Result<mekai::kimi::mcp::McpServers> {
    let path = mcp_config_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    } else {
        Ok(mekai::kimi::mcp::McpServers::default())
    }
}

fn save_mcp_servers(servers: &mekai::kimi::mcp::McpServers) -> Result<()> {
    let path = mcp_config_path();
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("")))?;
    let content = serde_json::to_string_pretty(servers)?;
    std::fs::write(path, content)?;
    Ok(())
}

async fn list_mcp_servers() -> Result<()> {
    let servers = load_mcp_servers()?;
    if servers.servers.is_empty() {
        println!("No MCP servers configured.");
    } else {
        for (name, config) in &servers.servers {
            println!("{name}: {} {}", config.command, config.args.join(" "));
        }
    }
    Ok(())
}
