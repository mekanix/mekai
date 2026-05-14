use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::kimi::error::Result;

pub struct WireServer {
    hub: Arc<crate::kimi::wire::WireHub>,
}

impl WireServer {
    pub fn new(hub: Arc<crate::kimi::wire::WireHub>) -> Self {
        Self { hub }
    }

    pub async fn serve_stdio(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();
        let mut stdout = stdout;

        let mut rx = self.hub.subscribe();

        loop {
            tokio::select! {
                line = lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            match self.handle_request(&line).await {
                                Ok(resp) => {
                                    stdout.write_all(resp.to_string().as_bytes()).await?;
                                    stdout.write_all(b"\n").await?;
                                    stdout.flush().await?;
                                }
                                Err(e) => {
                                    let resp = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "error": { "code": -32603, "message": e.to_string() },
                                        "id": null
                                    });
                                    stdout.write_all(resp.to_string().as_bytes()).await?;
                                    stdout.write_all(b"\n").await?;
                                    stdout.flush().await?;
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(e) => return Err(e.into()),
                    }
                }
                event = rx.recv() => {
                    if let Ok(event) = event {
                        let msg = serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "wire/event",
                            "params": event
                        });
                        stdout.write_all(msg.to_string().as_bytes()).await?;
                        stdout.write_all(b"\n").await?;
                        stdout.flush().await?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_request(&self, line: &str) -> Result<serde_json::Value> {
        let req: serde_json::Value = serde_json::from_str(line)?;
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);

        match method {
            "initialize" => Ok(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "protocolVersion": "2024-11-05", "capabilities": {} }
            })),
            "tools/list" => Ok(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": [] }
            })),
            "chat" => {
                let params = req.get("params").cloned().unwrap_or_default();
                let prompt = params.get("prompt").and_then(|p| p.as_str()).unwrap_or("");
                let _ = self
                    .hub
                    .send(crate::kimi::wire::types::WireEvent::StatusUpdate {
                        message: format!("User: {prompt}"),
                        timestamp: chrono::Utc::now(),
                    });
                let result = if let Some(llm) = crate::kimi::soul::tools::LLM_PROVIDER.get() {
                    let (tx, _rx) = tokio::sync::broadcast::channel(16);
                    let approval =
                        std::sync::Arc::new(crate::kimi::approval::ApprovalRuntime::new());
                    approval.set_yolo(true).await;
                    let mut soul = crate::kimi::soul::KimiSoul::new(
                        crate::kimi::soul::Agent {
                            name: "wire".to_string(),
                            system_prompt: "You are Mekai, a helpful CLI agent.".to_string(),
                            tools: vec![],
                        },
                        tx,
                        approval,
                        50000,
                        crate::kimi::config::LoopControl::default(),
                    );
                    match soul.run(prompt, std::sync::Arc::clone(llm)).await {
                        Ok(response) => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "response": response }
                        }),
                        Err(e) => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32603, "message": e.to_string() }
                        }),
                    }
                } else {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32603, "message": "LLM provider not available" }
                    })
                };
                Ok(result)
            }
            _ => Ok(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method not found: {method}") }
            })),
        }
    }
}
