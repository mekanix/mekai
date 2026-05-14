use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WireEvent {
    TurnBegin {
        turn: usize,
        timestamp: DateTime<Utc>,
    },
    StepBegin {
        step: usize,
        timestamp: DateTime<Utc>,
    },
    StepInterrupted {
        step: usize,
        reason: String,
        timestamp: DateTime<Utc>,
    },
    TurnEnd {
        turn: usize,
        timestamp: DateTime<Utc>,
    },
    RalphTurnBegin {
        iteration: usize,
        timestamp: DateTime<Utc>,
    },
    RalphTurnEnd {
        iteration: usize,
        timestamp: DateTime<Utc>,
    },
    StatusUpdate {
        message: String,
        timestamp: DateTime<Utc>,
    },
    CompactionBegin {
        timestamp: DateTime<Utc>,
    },
    CompactionEnd {
        timestamp: DateTime<Utc>,
    },
    MCPLoadingBegin {
        timestamp: DateTime<Utc>,
    },
    MCPLoadingEnd {
        timestamp: DateTime<Utc>,
    },
    Notification {
        level: String,
        message: String,
        timestamp: DateTime<Utc>,
    },
    SubagentEvent {
        subagent_id: String,
        event: Box<WireEvent>,
        timestamp: DateTime<Utc>,
    },
    BtwBegin {
        question: String,
        timestamp: DateTime<Utc>,
    },
    BtwEnd {
        answer: String,
        timestamp: DateTime<Utc>,
    },
    PlanDisplay {
        plan: String,
        timestamp: DateTime<Utc>,
    },
    ContentPart {
        content: String,
        timestamp: DateTime<Utc>,
    },
    ToolCall {
        call: crate::kimi::llm::ToolCall,
        timestamp: DateTime<Utc>,
    },
    ToolResult {
        call_id: String,
        result: crate::kimi::soul::tools::ToolResult,
        timestamp: DateTime<Utc>,
    },
    ApprovalRequest {
        request_id: String,
        tool_name: String,
        arguments: HashMap<String, serde_json::Value>,
        timestamp: DateTime<Utc>,
    },
    ApprovalResponse {
        request_id: String,
        approved: bool,
        timestamp: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub tool_name: String,
    pub arguments: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionRequest {
    pub id: String,
    pub question: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRequest {
    pub event: String,
    pub matcher: Option<String>,
    pub input_data: serde_json::Value,
}
