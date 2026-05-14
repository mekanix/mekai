use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::kimi::background::TaskStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundTask {
    pub id: String,
    pub status: TaskStatus,
    pub spec: TaskSpec,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub command: Option<String>,
    pub cwd: Option<PathBuf>,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub timeout_secs: Option<u64>,
}

impl BackgroundTask {
    pub fn new_bash(id: String, spec: TaskSpec, pid: Option<u32>) -> Self {
        Self {
            id,
            status: TaskStatus::Pending,
            spec,
            created_at: Utc::now(),
            completed_at: None,
            output: None,
            error: None,
            pid,
        }
    }

    pub fn new_agent(id: String) -> Self {
        Self {
            id,
            status: TaskStatus::Pending,
            spec: TaskSpec {
                command: None,
                cwd: None,
                env: None,
                timeout_secs: None,
            },
            created_at: Utc::now(),
            completed_at: None,
            output: None,
            error: None,
            pid: None,
        }
    }
}
