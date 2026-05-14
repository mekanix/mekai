use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::kimi::error::Result;
use crate::kimi::llm::Message;
use crate::kimi::metadata::{load_metadata, save_metadata};
use crate::kimi::session_state::SessionState;
use crate::kimi::wire::types::WireEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub work_dir: PathBuf,
    pub context_file: PathBuf,
    pub wire_file: PathBuf,
    pub state: SessionState,
    pub title: String,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn dir(&self) -> PathBuf {
        data_dir().join("sessions").join(&self.id)
    }

    pub async fn create(work_dir: &Path, id: Option<String>) -> Result<Self> {
        let id = id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let session = Session {
            id: id.clone(),
            work_dir: work_dir.to_path_buf(),
            context_file: data_dir().join("sessions").join(&id).join("context.jsonl"),
            wire_file: data_dir().join("sessions").join(&id).join("wire.jsonl"),
            state: SessionState::default(),
            title: format!("Session {}", &id[..8]),
            updated_at: Utc::now(),
        };
        std::fs::create_dir_all(session.dir())?;
        session.save()?;

        let mut meta = load_metadata()?;
        let wdm = meta.get_or_create_work_dir_meta(work_dir);
        wdm.last_session_id = Some(id);
        save_metadata(&meta)?;

        Ok(session)
    }

    pub async fn find(work_dir: &Path, id: &str) -> Result<Option<Self>> {
        let path = data_dir().join("sessions").join(id).join("session.json");
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)?;
        let mut session: Session = serde_json::from_str(&content)?;
        session.work_dir = work_dir.to_path_buf();
        Ok(Some(session))
    }

    pub async fn list(work_dir: &Path) -> Result<Vec<Self>> {
        let sessions_dir = data_dir().join("sessions");
        if !sessions_dir.exists() {
            return Ok(vec![]);
        }
        let mut sessions = vec![];
        for entry in std::fs::read_dir(sessions_dir)? {
            let entry = entry?;
            let path = entry.path().join("session.json");
            if path.exists() {
                let content = std::fs::read_to_string(path)?;
                if let Ok(mut session) = serde_json::from_str::<Session>(&content) {
                    session.work_dir = work_dir.to_path_buf();
                    sessions.push(session);
                }
            }
        }
        sessions.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(sessions)
    }

    pub async fn continue_(work_dir: &Path) -> Result<Option<Self>> {
        let meta = load_metadata()?;
        if let Some(wdm) = meta.get_work_dir_meta(work_dir)
            && let Some(ref id) = wdm.last_session_id
        {
            return Session::find(work_dir, id).await;
        }
        Ok(None)
    }

    pub async fn fork(&self, new_id: Option<String>) -> Result<Self> {
        let new_id = new_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let new_session = Session {
            id: new_id.clone(),
            work_dir: self.work_dir.clone(),
            context_file: data_dir()
                .join("sessions")
                .join(&new_id)
                .join("context.jsonl"),
            wire_file: data_dir().join("sessions").join(&new_id).join("wire.jsonl"),
            state: self.state.clone(),
            title: format!("Fork of {}", self.title),
            updated_at: Utc::now(),
        };
        std::fs::create_dir_all(new_session.dir())?;
        new_session.save()?;

        // Copy context.jsonl
        if self.context_file.exists() {
            let _ = std::fs::copy(&self.context_file, &new_session.context_file);
        }
        // Copy wire.jsonl
        if self.wire_file.exists() {
            let _ = std::fs::copy(&self.wire_file, &new_session.wire_file);
        }
        // Copy state.json
        let state_src = self.dir().join("state.json");
        if state_src.exists() {
            let _ = std::fs::copy(&state_src, new_session.dir().join("state.json"));
        }

        let mut meta = load_metadata()?;
        let wdm = meta.get_or_create_work_dir_meta(&self.work_dir);
        wdm.last_session_id = Some(new_id);
        save_metadata(&meta)?;

        Ok(new_session)
    }

    pub async fn delete(&self) -> Result<()> {
        let dir = self.dir();
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let path = self.dir().join("session.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn save_state(&self) -> Result<()> {
        // State is persisted inline in session.json; this is a lightweight alias
        self.save()
    }

    pub fn is_empty(&self) -> bool {
        if let Ok(meta) = std::fs::metadata(&self.context_file) {
            meta.len() == 0
        } else {
            true
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    pub fn append_context(&self, message: &Message) -> Result<()> {
        let line = serde_json::to_string(message)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.context_file)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    pub fn load_context(&self) -> Result<Vec<Message>> {
        if !self.context_file.exists() {
            return Ok(vec![]);
        }
        let content = std::fs::read_to_string(&self.context_file)?;
        let mut messages = vec![];
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<Message>(line) {
                messages.push(msg);
            }
        }
        Ok(messages)
    }

    pub fn append_wire_event(&self, event: &WireEvent) -> Result<()> {
        let line = serde_json::to_string(event)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.wire_file)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    pub fn generate_title(&mut self, first_message: &str) {
        if self.title.starts_with("Session ") {
            let trimmed = if first_message.len() > 40 {
                format!("{}...", &first_message[..40])
            } else {
                first_message.to_string()
            };
            self.title = trimmed;
        }
    }
}

fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mekai")
}
