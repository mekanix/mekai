use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::kimi::error::{MekaiError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub default_model: Option<String>,
    pub default_thinking: Option<bool>,
    pub default_yolo: Option<bool>,
    pub default_plan_mode: Option<bool>,
    pub theme: Option<Theme>,
    pub show_thinking_stream: Option<bool>,
    pub telemetry: Option<bool>,
    pub merge_all_available_skills: Option<bool>,
    pub extra_skill_dirs: Vec<PathBuf>,
    pub models: HashMap<String, LlmModel>,
    pub providers: HashMap<String, LlmProvider>,
    pub loop_control: LoopControl,
    pub background: BackgroundConfig,
    pub services: Services,
    pub mcp: McpConfig,
    pub hooks: Vec<HookDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Theme {
    pub user_color: Option<String>,
    pub assistant_color: Option<String>,
    pub error_color: Option<String>,
    pub btw_color: Option<String>,
    pub border_style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LlmModel {
    pub provider: String,
    pub model: String,
    pub max_context_size: Option<usize>,
    pub temperature: Option<f32>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LlmProvider {
    pub r#type: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoopControl {
    pub max_steps_per_turn: usize,
    pub max_retries_per_step: usize,
    pub max_ralph_iterations: i32,
    pub reserved_context_size: usize,
    pub compaction_trigger_ratio: f64,
}

impl Default for LoopControl {
    fn default() -> Self {
        Self {
            max_steps_per_turn: 500,
            max_retries_per_step: 3,
            max_ralph_iterations: 0,
            reserved_context_size: 50000,
            compaction_trigger_ratio: 0.85,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackgroundConfig {
    pub max_running_tasks: usize,
    pub agent_task_timeout_s: u64,
    pub keep_alive_on_exit: bool,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            max_running_tasks: 4,
            agent_task_timeout_s: 900,
            keep_alive_on_exit: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Services {
    #[serde(rename = "moonshot_search")]
    pub moonshot_search: Option<ServiceConfig>,
    #[serde(rename = "moonshot_fetch")]
    pub moonshot_fetch: Option<ServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub base_url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct McpConfig {
    pub client: McpClientConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpClientConfig {
    pub tool_call_timeout_ms: u64,
}

impl Default for McpClientConfig {
    fn default() -> Self {
        Self {
            tool_call_timeout_ms: 60000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDef {
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
    pub timeout: Option<u64>,
}

pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mekai")
        .join("config.toml")
}

pub fn load_config_from_file<P: AsRef<Path>>(path: P) -> Result<Config> {
    let content = std::fs::read_to_string(path.as_ref())?;
    load_config_from_string(&content)
}

pub fn load_config_from_string(content: &str) -> Result<Config> {
    let config: Config = toml::from_str(content)?;
    Ok(config)
}

pub fn load_config() -> Result<Config> {
    let path = default_config_path();
    if path.exists() {
        load_config_from_file(path)
    } else {
        Ok(Config::default())
    }
}

pub fn save_config<P: AsRef<Path>>(config: &Config, path: P) -> Result<()> {
    let content = toml::to_string_pretty(config)
        .map_err(|e| MekaiError::Other(format!("TOML serialize error: {e}")))?;
    std::fs::create_dir_all(path.as_ref().parent().unwrap_or(Path::new("")))?;
    std::fs::write(path.as_ref(), content)?;
    Ok(())
}
