use std::sync::Arc;

use async_trait::async_trait;

use crate::kimi::error::Result;
use crate::kimi::llm::{ChatProvider, Message};

#[async_trait]
pub trait CompactionStrategy: Send + Sync {
    async fn compact(&self, messages: &[Message]) -> Result<Vec<Message>>;
}

pub struct SimpleCompaction;

impl Default for SimpleCompaction {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleCompaction {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CompactionStrategy for SimpleCompaction {
    async fn compact(&self, messages: &[Message]) -> Result<Vec<Message>> {
        if messages.len() < 4 {
            return Ok(messages.to_vec());
        }
        let mut compacted = vec![];
        if let Some(first) = messages.first()
            && first.role == "system"
        {
            compacted.push(first.clone());
        }
        compacted.push(Message {
            role: "user".to_string(),
            content: "[Earlier conversation summarized]".to_string(),
            tool_calls: None,
            tool_call_id: None,
        });
        compacted.extend_from_slice(&messages[messages.len().saturating_sub(3)..]);
        Ok(compacted)
    }
}

pub struct LlmCompaction {
    llm: Arc<dyn ChatProvider>,
}

impl LlmCompaction {
    pub fn new(llm: Arc<dyn ChatProvider>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl CompactionStrategy for LlmCompaction {
    async fn compact(&self, messages: &[Message]) -> Result<Vec<Message>> {
        if messages.len() < 6 {
            return Ok(messages.to_vec());
        }

        let mut compacted = vec![];
        let mut split_idx = 0;

        // Keep system prompt
        if let Some(first) = messages.first()
            && first.role == "system"
        {
            compacted.push(first.clone());
            split_idx = 1;
        }

        // Keep last 3 messages, summarize the rest
        let keep_start = messages.len().saturating_sub(3);
        let to_summarize = &messages[split_idx..keep_start];

        if to_summarize.len() >= 2 {
            let summary_prompt = format!(
                "Summarize the following conversation concisely, preserving key facts, decisions, and action items:\n\n{}\n\nProvide a brief summary:",
                to_summarize
                    .iter()
                    .map(|m| format!("{}: {}", m.role, m.content))
                    .collect::<Vec<_>>()
                    .join("\n")
            );

            let summary = match self
                .llm
                .chat(vec![Message::user(summary_prompt)], None)
                .await
            {
                Ok(resp) => resp.message.content,
                Err(_) => "[Earlier conversation was compacted]".to_string(),
            };

            compacted.push(Message {
                role: "user".to_string(),
                content: format!("[Summary of earlier conversation]: {summary}"),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        compacted.extend_from_slice(&messages[keep_start..]);
        Ok(compacted)
    }
}
