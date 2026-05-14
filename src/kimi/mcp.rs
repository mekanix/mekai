use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout};

use crate::kimi::error::{MekaiError, Result};
use crate::kimi::llm::ToolDef;
use crate::kimi::soul::tools::Tool;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServers {
    #[serde(flatten)]
    pub servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

pub struct McpClient {
    stdin: tokio::sync::Mutex<ChildStdin>,
    stdout: tokio::sync::Mutex<tokio::io::BufReader<ChildStdout>>,
    request_id: std::sync::atomic::AtomicU64,
    #[allow(dead_code)]
    child: tokio::sync::Mutex<Child>,
    timeout: std::time::Duration,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl McpClient {
    pub async fn connect(config: &McpServerConfig) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(&config.command);
        cmd.args(&config.args);
        cmd.envs(&config.env);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let client = Self {
            stdin: tokio::sync::Mutex::new(stdin),
            stdout: tokio::sync::Mutex::new(tokio::io::BufReader::new(stdout)),
            request_id: std::sync::atomic::AtomicU64::new(1),
            child: tokio::sync::Mutex::new(child),
            timeout: std::time::Duration::from_secs(60),
        };

        // Send initialize request
        let _ = client
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "mekai", "version": env!("CARGO_PKG_VERSION") }
                }),
            )
            .await?;

        Ok(client)
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    async fn request(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let id = self
            .request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };
        let req_line = serde_json::to_string(&req)?;

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(req_line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        drop(stdin);

        let mut stdout = self.stdout.lock().await;
        let mut line = String::new();
        let read_result = tokio::time::timeout(self.timeout, stdout.read_line(&mut line)).await;
        drop(stdout);

        match read_result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(MekaiError::Mcp("MCP request timed out".into())),
        }

        let resp: JsonRpcResponse = serde_json::from_str(&line)?;
        if let Some(err) = resp.error {
            return Err(MekaiError::Mcp(format!("{}: {}", err.code, err.message)));
        }
        resp.result
            .ok_or_else(|| MekaiError::Mcp("Empty response".into()))
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolDef>> {
        let result = self.request("tools/list", serde_json::json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let mut defs = Vec::new();
        for tool in tools {
            if let Ok(def) = serde_json::from_value::<ToolDef>(tool) {
                defs.push(def);
            }
        }
        Ok(defs)
    }

    pub async fn call_tool(&self, name: &str, arguments: serde_json::Value) -> Result<String> {
        let result = self
            .request(
                "tools/call",
                serde_json::json!({
                    "name": name,
                    "arguments": arguments,
                }),
            )
            .await?;
        Ok(result.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct McpTool {
    pub client: Arc<McpClient>,
    pub tool_name: String,
    pub tool_description: String,
    pub tool_parameters: serde_json::Value,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters(&self) -> serde_json::Value {
        self.tool_parameters.clone()
    }

    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let arguments = serde_json::to_value(args)?;
        self.client.call_tool(&self.tool_name, arguments).await
    }
}
