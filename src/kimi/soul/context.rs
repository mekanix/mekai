use crate::kimi::llm::Message;

#[derive(Debug, Clone, Default)]
pub struct Context {
    pub messages: Vec<Message>,
    pub max_tokens: usize,
    pub reserved_tokens: usize,
}

impl Context {
    pub fn new(max_tokens: usize, reserved_tokens: usize) -> Self {
        Self {
            messages: vec![],
            max_tokens,
            reserved_tokens,
        }
    }

    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn estimated_tokens(&self) -> usize {
        // Very rough estimate: 4 chars per token
        self.messages.iter().map(|m| m.content.len() / 4).sum()
    }

    pub fn needs_compaction(&self, trigger_ratio: f64) -> bool {
        let effective_max = self.max_tokens.saturating_sub(self.reserved_tokens);
        if effective_max == 0 {
            return false;
        }
        (self.estimated_tokens() as f64 / effective_max as f64) >= trigger_ratio
    }
}
