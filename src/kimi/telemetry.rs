use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::kimi::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub payload: serde_json::Value,
}

pub struct TelemetrySink {
    enabled: bool,
    log_dir: PathBuf,
    buffer: Vec<TelemetryEvent>,
}

impl TelemetrySink {
    pub fn new(enabled: bool) -> Self {
        let log_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mekai")
            .join("telemetry");
        Self {
            enabled,
            log_dir,
            buffer: vec![],
        }
    }

    pub fn emit(&mut self, event: TelemetryEvent) {
        if !self.enabled {
            return;
        }
        self.buffer.push(event);
    }

    pub fn flush(&mut self) -> Result<()> {
        if !self.enabled || self.buffer.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.log_dir)?;
        let path = self
            .log_dir
            .join(format!("{}.jsonl", Utc::now().format("%Y-%m-%d")));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        for event in &self.buffer {
            let line = serde_json::to_string(event)?;
            use std::io::Write;
            writeln!(file, "{line}")?;
        }
        self.buffer.clear();
        Ok(())
    }
}
