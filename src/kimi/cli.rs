use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "mekai")]
#[command(about = "Mekai, your next CLI agent.")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(
    help_template = "{before-help}{name} {version}\n{about-with-newline}\n{usage-heading} {usage}\n\n{all-args}{after-help}"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(short, long, help = "Print verbose information")]
    pub verbose: bool,

    #[arg(long, help = "Log debug information")]
    pub debug: bool,

    #[arg(
        short,
        long,
        help = "Working directory for the agent",
        value_name = "DIR"
    )]
    pub work_dir: Option<PathBuf>,

    #[arg(short = 'S', long = "session", help = "Resume a session")]
    pub resume: Option<String>,

    #[arg(short = 'C', long, help = "Continue the previous session")]
    pub continue_: bool,

    #[arg(short, long, help = "User prompt to the agent")]
    pub prompt: Option<String>,

    #[arg(long, help = "Run in print mode (non-interactive)")]
    pub print: bool,

    #[arg(long, help = "Run as ACP server")]
    pub acp: bool,

    #[arg(long, help = "Run as Wire server (experimental)")]
    pub wire: bool,

    #[arg(long, help = "Automatically approve all actions")]
    pub yolo: bool,

    #[arg(long, help = "Start in plan mode")]
    pub plan: bool,

    #[arg(short, long, help = "LLM model to use")]
    pub model: Option<String>,

    #[arg(long, help = "Enable thinking mode")]
    pub thinking: Option<bool>,

    #[arg(long, help = "Config TOML/JSON string to load")]
    pub config: Option<String>,

    #[arg(long, help = "Config TOML/JSON file to load")]
    pub config_file: Option<PathBuf>,

    #[arg(long, help = "Custom agent specification file")]
    pub agent_file: Option<PathBuf>,

    #[arg(long, help = "MCP config file to load", value_name = "FILE")]
    pub mcp_config_file: Vec<PathBuf>,

    #[arg(long, help = "MCP config JSON to load", value_name = "JSON")]
    pub mcp_config: Vec<String>,

    #[arg(long, help = "Custom skills directories", value_name = "DIR")]
    pub skills_dir: Vec<PathBuf>,

    #[arg(
        long,
        help = "Add an additional directory to the workspace scope",
        value_name = "DIR"
    )]
    pub add_dir: Vec<PathBuf>,

    #[arg(long, help = "Maximum number of steps in one turn")]
    pub max_steps_per_turn: Option<usize>,

    #[arg(long, help = "Maximum number of retries in one step")]
    pub max_retries_per_step: Option<usize>,

    #[arg(long, help = "Extra iterations after the first turn in Ralph mode")]
    pub max_ralph_iterations: Option<i32>,

    #[arg(long, help = "Input format to use")]
    pub input_format: Option<String>,

    #[arg(long, help = "Output format to use")]
    pub output_format: Option<String>,

    #[arg(long, help = "Only print the final assistant message (print UI)")]
    pub final_message_only: bool,

    #[arg(
        long,
        help = "Alias for --print --output-format text --final-message-only"
    )]
    pub quiet: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(about = "Run Toad TUI backed by Mekai CLI ACP server")]
    Term {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(about = "Export session diagnostics")]
    Export {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    #[command(about = "Show system information")]
    Info,
    #[command(about = "Manage MCP servers")]
    Mcp {
        #[command(subcommand)]
        command: Option<McpCommands>,
    },
    #[command(about = "Manage plugins")]
    Plugin {
        #[command(subcommand)]
        command: Option<PluginCommands>,
    },
    #[command(about = "Run the tracing visualizer")]
    Vis,
}

#[derive(Subcommand, Debug)]
pub enum McpCommands {
    #[command(about = "List MCP servers")]
    List,
    #[command(about = "Add an MCP server")]
    Add {
        name: String,
        #[arg(long)]
        command: String,
    },
    #[command(about = "Remove an MCP server")]
    Remove { name: String },
}

#[derive(Subcommand, Debug)]
pub enum PluginCommands {
    #[command(about = "List installed plugins")]
    List,
    #[command(about = "Install a plugin")]
    Install { source: String },
    #[command(about = "Remove a plugin")]
    Remove { name: String },
}
