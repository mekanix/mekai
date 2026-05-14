use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    #[serde(default)]
    pub additional_dirs: Vec<String>,
    #[serde(default)]
    pub approval_settings: ApprovalSettings,
    #[serde(default)]
    pub plan_mode: bool,
    #[serde(default)]
    pub todos: Vec<Todo>,
    #[serde(default)]
    pub archive: Vec<ArchivedTurn>,
    #[serde(default)]
    pub custom_data: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApprovalSettings {
    pub yolo: bool,
    pub per_action: HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: String,
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedTurn {
    pub turn: usize,
    pub summary: String,
}
