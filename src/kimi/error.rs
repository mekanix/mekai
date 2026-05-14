use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum MekaiError {
    #[error("config error: {0}")]
    Config(String),

    #[error("session error: {0}")]
    Session(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("LLM provider error: {0}")]
    Llm(String),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("approval denied")]
    ApprovalDenied,

    #[error("tool error: {0}")]
    Tool(String),

    #[error("background task error: {0}")]
    BackgroundTask(String),

    #[error("subagent error: {0}")]
    Subagent(String),

    #[error("wire error: {0}")]
    Wire(String),

    #[error("hook error: {0}")]
    Hook(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("skill error: {0}")]
    Skill(String),

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("ACP error: {0}")]
    Acp(String),

    #[error("session not found: {id}")]
    SessionNotFound { id: String },

    #[error("model not found: {name}")]
    ModelNotFound { name: String },

    #[error("provider not found: {name}")]
    ProviderNotFound { name: String },

    #[error("walkdir error: {0}")]
    Walkdir(#[from] walkdir::Error),

    #[error("invalid path: {0}")]
    InvalidPath(PathBuf),

    #[error("unknown command: {0}")]
    UnknownCommand(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, MekaiError>;
