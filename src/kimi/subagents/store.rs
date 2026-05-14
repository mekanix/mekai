use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SubagentStore {
    agents: HashMap<String, SubagentRecord>,
}

#[derive(Debug, Clone)]
pub struct SubagentRecord {
    pub id: String,
    pub agent_type: String,
    pub status: String,
}

impl SubagentStore {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    pub fn insert(&mut self, record: SubagentRecord) {
        self.agents.insert(record.id.clone(), record);
    }

    pub fn get(&self, id: &str) -> Option<&SubagentRecord> {
        self.agents.get(id)
    }

    pub fn remove(&mut self, id: &str) -> Option<SubagentRecord> {
        self.agents.remove(id)
    }

    pub fn list(&self) -> Vec<&SubagentRecord> {
        self.agents.values().collect()
    }
}

impl Default for SubagentStore {
    fn default() -> Self {
        Self::new()
    }
}
