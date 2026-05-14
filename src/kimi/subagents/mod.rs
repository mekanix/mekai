pub mod runner;
pub mod store;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentType {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub system_prompt: String,
}

#[derive(Debug, Clone)]
pub struct LaborMarket {
    pub types: HashMap<String, SubagentType>,
}

impl LaborMarket {
    pub fn new() -> Self {
        let mut types = HashMap::new();
        types.insert(
            "explore".to_string(),
            SubagentType {
                name: "explore".to_string(),
                description: "Explore a codebase quickly".to_string(),
                tools: vec!["ReadFile".to_string(), "Shell".to_string()],
                system_prompt: "You are an explore agent. Be concise.".to_string(),
            },
        );
        types.insert(
            "coder".to_string(),
            SubagentType {
                name: "coder".to_string(),
                description: "Write and edit code".to_string(),
                tools: vec![
                    "ReadFile".to_string(),
                    "WriteFile".to_string(),
                    "Shell".to_string(),
                ],
                system_prompt: "You are a coding agent. Write clean, correct code.".to_string(),
            },
        );
        types.insert(
            "plan".to_string(),
            SubagentType {
                name: "plan".to_string(),
                description: "Create implementation plans".to_string(),
                tools: vec!["ReadFile".to_string(), "Think".to_string()],
                system_prompt: "You are a planning agent. Create detailed plans.".to_string(),
            },
        );
        Self { types }
    }
}

impl Default for LaborMarket {
    fn default() -> Self {
        Self::new()
    }
}
