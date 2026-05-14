use std::io::{self, BufRead};
use std::sync::Arc;

use futures::StreamExt;

use crate::kimi::error::Result;
use crate::kimi::llm::ChatProvider;
use crate::kimi::soul::{KimiSoul, SoulEvent};

pub struct Print {
    pub input_format: String,
    pub output_format: String,
    pub final_message_only: bool,
}

impl Print {
    pub fn new(input_format: String, output_format: String, final_message_only: bool) -> Self {
        Self {
            input_format,
            output_format,
            final_message_only,
        }
    }

    pub async fn run(
        &mut self,
        llm: Arc<dyn ChatProvider>,
        soul: &mut KimiSoul,
        prompt: Option<String>,
    ) -> Result<i32> {
        let input = if let Some(p) = prompt {
            p
        } else {
            self.read_stdin()?
        };

        if self.output_format == "stream-json" {
            println!(
                "{{\"type\": \"turn_begin\", \"input\": {}}}",
                serde_json::json!(&input)
            );

            let mut stream = soul.run_streaming(input, llm);
            let mut full_response = String::new();

            while let Some(event) = stream.next().await {
                match event {
                    SoulEvent::Token(text) => {
                        full_response.push_str(&text);
                        println!(
                            "{{\"type\": \"token\", \"text\": {}}}",
                            serde_json::json!(&text)
                        );
                    }
                    SoulEvent::ToolCall(call) => {
                        println!(
                            "{{\"type\": \"tool_call\", \"call\": {}}}",
                            serde_json::to_string(&call).unwrap_or_default()
                        );
                    }
                    SoulEvent::ToolResult { call_id, output } => {
                        println!(
                            "{{\"type\": \"tool_result\", \"call_id\": {:?}, \"output\": {}}}",
                            call_id,
                            serde_json::json!(&output)
                        );
                    }
                    SoulEvent::Plan(plan) => {
                        println!(
                            "{{\"type\": \"plan\", \"plan\": {}}}",
                            serde_json::json!(&plan)
                        );
                    }
                    SoulEvent::Done(text) => {
                        println!(
                            "{{\"type\": \"done\", \"text\": {}}}",
                            serde_json::json!(&text)
                        );
                    }
                    SoulEvent::Error(err) => {
                        eprintln!(
                            "{{\"type\": \"error\", \"error\": {}}}",
                            serde_json::json!(&err)
                        );
                    }
                    SoulEvent::ApprovalNeeded { tool_name, args } => {
                        println!(
                            "{{\"type\": \"approval_needed\", \"tool\": {:?}, \"args\": {}}}",
                            tool_name,
                            serde_json::json!(&args)
                        );
                    }
                }
            }

            println!("{{\"type\": \"turn_end\"}}");

            if self.final_message_only && !full_response.is_empty() {
                // final_message_only with stream-json is a bit contradictory,
                // but we already streamed JSON events above
            }
        } else {
            let response = soul.run(&input, llm).await?;

            if self.final_message_only {
                println!("{response}");
            } else {
                println!("Assistant: {response}");
            }
        }

        Ok(0)
    }

    fn read_stdin(&self) -> Result<String> {
        let stdin = io::stdin();
        let mut lines = vec![];
        for line in stdin.lock().lines() {
            lines.push(line?);
        }
        let raw = lines.join("\n");
        if self.input_format == "json"
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw)
        {
            if let Some(prompt) = json.get("prompt").and_then(|v| v.as_str()) {
                return Ok(prompt.to_string());
            }
            if let Some(content) = json.get("content").and_then(|v| v.as_str()) {
                return Ok(content.to_string());
            }
        }
        Ok(raw)
    }
}
