use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;

use crate::kimi::error::Result;

#[derive(Debug, Clone)]
pub struct AcpSession {
    pub id: String,
    pub context: Vec<crate::kimi::llm::Message>,
}

pub struct AcpServer {
    sessions: Arc<RwLock<HashMap<String, AcpSession>>>,
}

impl AcpServer {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn run(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();
        let mut stdout = stdout;

        println!("ACP server ready.");

        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(req) = serde_json::from_str::<serde_json::Value>(&line) {
                let resp = self.handle_request(req).await;
                let resp_line = serde_json::to_string(&resp)?;
                stdout.write_all(resp_line.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
            }
        }

        Ok(())
    }

    async fn handle_request(&self, req: serde_json::Value) -> serde_json::Value {
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);

        match method {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "protocolVersion": "2024-11-05", "capabilities": {} }
            }),
            "sessions/list" => {
                let sessions = self.sessions.read().await;
                let list: Vec<_> = sessions
                    .values()
                    .map(|s| {
                        serde_json::json!({
                            "id": s.id,
                            "message_count": s.context.len()
                        })
                    })
                    .collect();
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "sessions": list }
                })
            }
            "sessions/create" => {
                let session_id = uuid::Uuid::new_v4().to_string();
                let session = AcpSession {
                    id: session_id.clone(),
                    context: vec![],
                };
                self.sessions
                    .write()
                    .await
                    .insert(session_id.clone(), session);
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "id": session_id }
                })
            }
            "prompts/get" => {
                let params = req.get("params").cloned().unwrap_or_default();
                let session_id = params
                    .get("session_id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let sessions = self.sessions.read().await;
                if let Some(session) = sessions.get(session_id) {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "messages": session.context,
                        }
                    })
                } else {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32000, "message": "Session not found" }
                    })
                }
            }
            _ => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method not found: {method}") }
            }),
        }
    }
}

impl Default for AcpServer {
    fn default() -> Self {
        Self::new()
    }
}
