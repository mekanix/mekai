use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::kimi::background::manager::BackgroundTaskManager;
use crate::kimi::error::Result;
use crate::kimi::llm::ChatProvider;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
}

pub(crate) static BG_MANAGER: OnceLock<BackgroundTaskManager> = OnceLock::new();
pub(crate) static LLM_PROVIDER: OnceLock<Arc<dyn ChatProvider>> = OnceLock::new();
pub(crate) static SUBAGENT_STORE: OnceLock<
    Arc<tokio::sync::RwLock<crate::kimi::subagents::store::SubagentStore>>,
> = OnceLock::new();
pub(crate) static SERVICE_SEARCH: OnceLock<Option<crate::kimi::config::ServiceConfig>> =
    OnceLock::new();
pub(crate) static SERVICE_FETCH: OnceLock<Option<crate::kimi::config::ServiceConfig>> =
    OnceLock::new();

pub fn init_shared_resources(llm: Arc<dyn ChatProvider>, bg_manager: BackgroundTaskManager) {
    let _ = LLM_PROVIDER.set(llm);
    let _ = BG_MANAGER.set(bg_manager);
    let _ = SUBAGENT_STORE.set(Arc::new(tokio::sync::RwLock::new(
        crate::kimi::subagents::store::SubagentStore::new(),
    )));
}

pub fn init_services(
    search: Option<crate::kimi::config::ServiceConfig>,
    fetch: Option<crate::kimi::config::ServiceConfig>,
) {
    let _ = SERVICE_SEARCH.set(search);
    let _ = SERVICE_FETCH.set(fetch);
}

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "ReadFile"
    }
    fn description(&self) -> &str {
        "Read the contents of a file."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file" }
            },
            "required": ["path"]
        })
    }
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::kimi::error::MekaiError::Tool("Missing 'path' argument".into())
        })?;
        let content = tokio::fs::read_to_string(path).await?;
        Ok(content)
    }
}

pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "WriteFile"
    }
    fn description(&self) -> &str {
        "Write content to a file."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        })
    }
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::kimi::error::MekaiError::Tool("Missing 'path' argument".into())
        })?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::kimi::error::MekaiError::Tool("Missing 'content' argument".into())
            })?;
        tokio::fs::write(path, content).await?;
        Ok("File written successfully".to_string())
    }
}

pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "Shell"
    }
    fn description(&self) -> &str {
        "Execute a shell command."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" }
            },
            "required": ["command"]
        })
    }
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::kimi::error::MekaiError::Tool("Missing 'command' argument".into())
            })?;
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            Ok(stdout.to_string())
        } else {
            Ok(format!("Error:\n{stderr}\n{stdout}"))
        }
    }
}

pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }
    fn description(&self) -> &str {
        "Search the web for information."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" }
            },
            "required": ["query"]
        })
    }
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let query = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::kimi::error::MekaiError::Tool("Missing 'query' argument".into())
        })?;
        if let Some(Some(config)) = SERVICE_SEARCH.get() {
            let client = reqwest::Client::new();
            let mut req = client.get(format!("{}/search?q={}", config.base_url, query));
            if let Some(ref key) = config.api_key {
                req = req.header("Authorization", format!("Bearer {key}"));
            }
            let resp = req.send().await?;
            let text = resp.text().await?;
            Ok(text)
        } else {
            Ok(format!(
                "Web search results for: {query}\n[Search service not configured — set services.moonshot_search in config]"
            ))
        }
    }
}

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }
    fn description(&self) -> &str {
        "Fetch the content of a URL."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to fetch" }
            },
            "required": ["url"]
        })
    }
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::kimi::error::MekaiError::Tool("Missing 'url' argument".into()))?;
        let client = reqwest::Client::new();
        let resp = client.get(url).send().await?;
        let text = resp.text().await?;
        Ok(text)
    }
}

pub struct ThinkTool;

#[async_trait]
impl Tool for ThinkTool {
    fn name(&self) -> &str {
        "Think"
    }
    fn description(&self) -> &str {
        "Think through a problem step by step."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "thoughts": { "type": "string", "description": "Your thoughts" }
            },
            "required": ["thoughts"]
        })
    }
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let thoughts = args
            .get("thoughts")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::kimi::error::MekaiError::Tool("Missing 'thoughts' argument".into())
            })?;
        Ok(format!("Thoughts recorded: {thoughts}"))
    }
}

pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "AskUser"
    }
    fn description(&self) -> &str {
        "Ask the user a question and wait for their response."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "Question to ask" }
            },
            "required": ["question"]
        })
    }
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::kimi::error::MekaiError::Tool("Missing 'question' argument".into())
            })?;
        println!("{question}");
        let mut buffer = String::new();
        std::io::stdin().read_line(&mut buffer)?;
        Ok(buffer.trim().to_string())
    }
}

pub struct AgentTool;

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }
    fn description(&self) -> &str {
        "Spawn a subagent to perform a specialized task. Available types: explore, coder, plan."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "type": { "type": "string", "description": "Subagent type (explore, coder, plan)" },
                "task": { "type": "string", "description": "Task description for the subagent" }
            },
            "required": ["type", "task"]
        })
    }
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let agent_type = args.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::kimi::error::MekaiError::Tool("Missing 'type' argument".into())
        })?;
        let task = args.get("task").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::kimi::error::MekaiError::Tool("Missing 'task' argument".into())
        })?;

        let market = crate::kimi::subagents::LaborMarket::new();
        let Some(subagent_type) = market.types.get(agent_type) else {
            return Ok(format!(
                "Unknown subagent type: {agent_type}. Available: explore, coder, plan"
            ));
        };

        let Some(llm) = LLM_PROVIDER.get() else {
            return Ok("LLM provider not available for subagent".to_string());
        };

        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let approval = Arc::new(crate::kimi::approval::ApprovalRuntime::new());
        approval.set_yolo(true).await;

        let subagent_tools: Vec<crate::kimi::llm::ToolDef> = builtin_tools()
            .into_iter()
            .filter(|t| subagent_type.tools.contains(&t.name().to_string()))
            .map(|t| crate::kimi::llm::ToolDef {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
            })
            .collect();

        let agent = crate::kimi::soul::Agent {
            name: subagent_type.name.clone(),
            system_prompt: format!(
                "{system}\n\nYou are a specialized subagent. Your task: {task}\n\nBe concise. Focus only on the assigned task.",
                system = subagent_type.system_prompt
            ),
            tools: subagent_tools,
        };

        let mut subagent_soul = crate::kimi::soul::KimiSoul::new(
            agent,
            tx,
            approval,
            100000,
            crate::kimi::config::LoopControl::default(),
        );

        let id = uuid::Uuid::new_v4().to_string();
        if let Some(store) = SUBAGENT_STORE.get() {
            store
                .write()
                .await
                .insert(crate::kimi::subagents::store::SubagentRecord {
                    id: id.clone(),
                    agent_type: subagent_type.name.clone(),
                    status: "running".to_string(),
                });
        }
        match subagent_soul.run(task, Arc::clone(llm)).await {
            Ok(result) => {
                if let Some(store) = SUBAGENT_STORE.get() {
                    store
                        .write()
                        .await
                        .insert(crate::kimi::subagents::store::SubagentRecord {
                            id: id.clone(),
                            agent_type: subagent_type.name.clone(),
                            status: "completed".to_string(),
                        });
                }
                Ok(format!(
                    "Subagent '{}' completed task:\n---\n{task}\n---\nResult:\n{result}",
                    subagent_type.name
                ))
            }
            Err(e) => {
                if let Some(store) = SUBAGENT_STORE.get() {
                    store
                        .write()
                        .await
                        .insert(crate::kimi::subagents::store::SubagentRecord {
                            id: id.clone(),
                            agent_type: subagent_type.name.clone(),
                            status: "failed".to_string(),
                        });
                }
                Ok(format!("Subagent '{}' failed: {e}", subagent_type.name))
            }
        }
    }
}

pub struct BackgroundTaskTool;

#[async_trait]
impl Tool for BackgroundTaskTool {
    fn name(&self) -> &str {
        "BackgroundTask"
    }
    fn description(&self) -> &str {
        "Run a shell command in the background. Returns a task ID to check status later."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to run in background" },
                "timeout": { "type": "integer", "description": "Timeout in seconds (default: 300)" }
            },
            "required": ["command"]
        })
    }
    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::kimi::error::MekaiError::Tool("Missing 'command' argument".into())
            })?;
        let timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(300);

        let manager = BG_MANAGER.get().cloned().unwrap_or_else(|| {
            BackgroundTaskManager::new(4, std::time::Duration::from_secs(900), false)
        });

        let spec = crate::kimi::background::tasks::TaskSpec {
            command: Some(command.to_string()),
            cwd: Some(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            env: None,
            timeout_secs: Some(timeout),
        };
        let id = manager.create_bash_task(spec).await?;
        Ok(format!(
            "Background task started with ID: {id}\nUse /tasks or ListTasks to check status."
        ))
    }
}

pub struct ListTasksTool;

#[async_trait]
impl Tool for ListTasksTool {
    fn name(&self) -> &str {
        "ListTasks"
    }
    fn description(&self) -> &str {
        "List all background tasks and their status."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }
    async fn execute(&self, _args: HashMap<String, serde_json::Value>) -> Result<String> {
        let manager = BG_MANAGER.get().cloned().unwrap_or_else(|| {
            BackgroundTaskManager::new(4, std::time::Duration::from_secs(900), false)
        });
        let tasks = manager.list_tasks().await;
        if tasks.is_empty() {
            return Ok("No background tasks.".to_string());
        }
        let mut output = String::from("Background tasks:\n");
        for task in tasks {
            output.push_str(&format!(
                "  {} - {:?} - {}\n",
                &task.id[..8],
                task.status,
                task.spec.command.as_deref().unwrap_or("agent task")
            ));
        }
        Ok(output)
    }
}

pub fn builtin_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(FileReadTool),
        Arc::new(FileWriteTool),
        Arc::new(ShellTool),
        Arc::new(WebSearchTool),
        Arc::new(WebFetchTool),
        Arc::new(ThinkTool),
        Arc::new(AskUserTool),
        Arc::new(AgentTool),
        Arc::new(BackgroundTaskTool),
        Arc::new(ListTasksTool),
    ]
}
