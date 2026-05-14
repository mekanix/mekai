use std::collections::VecDeque;

use crate::kimi::llm::Message;

#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub id: String,
    pub messages: Vec<Message>,
    pub description: String,
}

pub struct DenwaRenji {
    checkpoints: VecDeque<Checkpoint>,
    max_checkpoints: usize,
}

impl DenwaRenji {
    pub fn new(max_checkpoints: usize) -> Self {
        Self {
            checkpoints: VecDeque::new(),
            max_checkpoints,
        }
    }

    pub fn checkpoint(&mut self, messages: Vec<Message>, description: impl Into<String>) -> String {
        let id = format!("checkpoint-{}", self.checkpoints.len());
        self.checkpoints.push_back(Checkpoint {
            id: id.clone(),
            messages,
            description: description.into(),
        });
        while self.checkpoints.len() > self.max_checkpoints {
            self.checkpoints.pop_front();
        }
        id
    }

    pub fn rollback(&self, id: &str) -> Option<Vec<Message>> {
        self.checkpoints
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.messages.clone())
    }

    pub fn list(&self) -> Vec<&Checkpoint> {
        self.checkpoints.iter().collect()
    }

    pub fn latest(&self) -> Option<&Checkpoint> {
        self.checkpoints.back()
    }
}

impl Default for DenwaRenji {
    fn default() -> Self {
        Self::new(10)
    }
}
