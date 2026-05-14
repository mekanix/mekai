use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::kimi::error::{MekaiError, Result};
use crate::kimi::llm::ToolDef;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    pub name: String,
    pub version: String,
    pub description: String,
    pub system_prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking: Option<bool>,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl AgentSpec {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let spec: AgentSpec = serde_yaml::from_str(&content)
            .map_err(|e| MekaiError::Config(format!("Invalid agent spec: {e}")))?;
        Ok(spec)
    }

    pub fn builtin_default() -> Self {
        Self {
            name: "default".to_string(),
            version: "1.0".to_string(),
            description: "Default Mekai agent".to_string(),
            system_prompt: "You are Mekai, a helpful CLI agent.".to_string(),
            tools: vec![],
            model: None,
            thinking: None,
            extra: HashMap::new(),
        }
    }
}

pub fn resolve_tools(tools: &[String], available: &[ToolDef]) -> Vec<ToolDef> {
    if tools.is_empty() {
        return available.to_vec();
    }
    let mut resolved = Vec::new();
    for name in tools {
        if let Some(tool) = available.iter().find(|t| &t.name == name) {
            resolved.push(tool.clone());
        }
    }
    resolved
}

pub fn find_agent_file(work_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        work_dir.join(".mekai/agent.yaml"),
        work_dir.join(".mekai/agent.yml"),
        work_dir.join("agent.yaml"),
        work_dir.join("agent.yml"),
    ];
    for path in &candidates {
        if path.exists() {
            return Some(path.clone());
        }
    }
    None
}
