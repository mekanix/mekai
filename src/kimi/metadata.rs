use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::kimi::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Metadata {
    #[serde(default)]
    pub work_dirs: HashMap<String, WorkDirMeta>,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkDirMeta {
    pub last_session_id: Option<String>,
    #[serde(default)]
    pub custom_data: HashMap<String, serde_json::Value>,
}

impl Metadata {
    pub fn get_work_dir_meta(&self, path: &Path) -> Option<&WorkDirMeta> {
        let key = path.canonicalize().ok()?.to_string_lossy().to_string();
        self.work_dirs.get(&key)
    }

    pub fn get_or_create_work_dir_meta(&mut self, path: &Path) -> &mut WorkDirMeta {
        let key = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string();
        self.work_dirs.entry(key).or_default()
    }
}

fn metadata_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mekai")
        .join("metadata.json")
}

pub fn load_metadata() -> Result<Metadata> {
    let path = metadata_path();
    if path.exists() {
        let content = std::fs::read_to_string(path)?;
        let meta: Metadata = serde_json::from_str(&content)?;
        Ok(meta)
    } else {
        Ok(Metadata::default())
    }
}

pub fn save_metadata(meta: &Metadata) -> Result<()> {
    let path = metadata_path();
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("")))?;
    let content = serde_json::to_string_pretty(meta)?;
    std::fs::write(path, content)?;
    Ok(())
}
