use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::kimi::error::Result;
use crate::kimi::soul::tools::Tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    pub name: String,
    pub version: String,
    pub description: String,
    pub spec: PluginSpec,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSpec {
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub command: String,
}

pub struct PluginManager {
    plugins_dir: PathBuf,
}

impl PluginManager {
    pub fn new() -> Self {
        let plugins_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mekai")
            .join("plugins");
        Self { plugins_dir }
    }

    pub fn list_plugins(&self) -> Result<Vec<Plugin>> {
        let mut plugins = vec![];
        if !self.plugins_dir.exists() {
            return Ok(plugins);
        }
        for entry in std::fs::read_dir(&self.plugins_dir)? {
            let entry = entry?;
            let plugin_json = entry.path().join("plugin.json");
            if plugin_json.exists() {
                let content = std::fs::read_to_string(plugin_json)?;
                let spec: PluginSpec = serde_json::from_str(&content)?;
                let name = entry.file_name().to_string_lossy().to_string();
                plugins.push(Plugin {
                    name: name.clone(),
                    version: spec
                        .config
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0.0.1")
                        .to_string(),
                    description: spec
                        .config
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    spec,
                    dir: entry.path(),
                });
            }
        }
        Ok(plugins)
    }

    pub async fn install(&self, source: &str) -> Result<()> {
        tracing::info!("Installing plugin from {source}...");
        std::fs::create_dir_all(&self.plugins_dir)?;

        let name = if source.starts_with("http://") || source.starts_with("https://") {
            // Download zip/tar.gz and extract
            let response = reqwest::get(source).await?;
            let bytes = response.bytes().await?;
            let name = source.split('/').next_back().unwrap_or("plugin");
            let name = name
                .strip_suffix(".zip")
                .or_else(|| name.strip_suffix(".tar.gz"))
                .unwrap_or(name);
            let dest = self.plugins_dir.join(name);
            if dest.exists() {
                return Err(crate::kimi::error::MekaiError::Other(format!(
                    "Plugin {name} already exists"
                )));
            }
            if source.ends_with(".zip") {
                extract_zip(&bytes, &dest)?;
            } else if source.ends_with(".tar.gz") {
                extract_tar_gz(&bytes, &dest)?;
            } else {
                return Err(crate::kimi::error::MekaiError::Other(
                    "Unsupported archive format. Use .zip or .tar.gz".into(),
                ));
            }
            name.to_string()
        } else if source.contains(':') || source.starts_with("git@") {
            // Clone git repo
            let name = source
                .split('/')
                .next_back()
                .unwrap_or("plugin")
                .strip_suffix(".git")
                .unwrap_or("plugin");
            let dest = self.plugins_dir.join(name);
            if dest.exists() {
                return Err(crate::kimi::error::MekaiError::Other(format!(
                    "Plugin {name} already exists"
                )));
            }
            let output = tokio::process::Command::new("git")
                .args(["clone", source, dest.to_str().unwrap_or(".")])
                .output()
                .await?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(crate::kimi::error::MekaiError::Other(format!(
                    "git clone failed: {stderr}"
                )));
            }
            name.to_string()
        } else {
            // Local path - copy
            let src = PathBuf::from(source);
            let name = src
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("plugin")
                .to_string();
            let dest = self.plugins_dir.join(&name);
            if dest.exists() {
                return Err(crate::kimi::error::MekaiError::Other(format!(
                    "Plugin {name} already exists"
                )));
            }
            if src.is_dir() {
                copy_dir_all(&src, &dest)?;
            } else {
                return Err(crate::kimi::error::MekaiError::Other(
                    "Local plugin must be a directory".into(),
                ));
            }
            name
        };

        println!("Installed plugin: {name}");
        Ok(())
    }

    pub async fn remove(&self, name: &str) -> Result<()> {
        let path = self.plugins_dir.join(name);
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PluginTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub command: String,
    pub dir: PathBuf,
}

#[async_trait]
impl Tool for PluginTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> serde_json::Value {
        self.parameters.clone()
    }

    async fn execute(&self, args: HashMap<String, serde_json::Value>) -> Result<String> {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&self.command);
        cmd.current_dir(&self.dir);

        let args_json = serde_json::to_string(&args)?;
        cmd.env("MEKAI_TOOL_ARGS", &args_json);

        let output = cmd.output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            return Err(crate::kimi::error::MekaiError::Tool(format!(
                "Plugin {} failed: {stderr}",
                self.name
            )));
        }

        Ok(stdout.trim().to_string())
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_zip(data: &[u8], dest: &PathBuf) -> Result<()> {
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| crate::kimi::error::MekaiError::Other(format!("ZIP error: {e}")))?;
    std::fs::create_dir_all(dest)?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| crate::kimi::error::MekaiError::Other(format!("ZIP error: {e}")))?;
        let outpath = dest.join(file.name());
        if file.name().ends_with('/') {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }
    Ok(())
}

fn extract_tar_gz(data: &[u8], dest: &PathBuf) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(data);
    let mut archive = tar::Archive::new(decoder);
    std::fs::create_dir_all(dest)?;
    archive
        .unpack(dest)
        .map_err(|e| crate::kimi::error::MekaiError::Other(format!("TAR error: {e}")))?;
    Ok(())
}

fn copy_dir_all(src: &PathBuf, dst: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}
