use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::kimi::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub skill_type: SkillType,
    pub source: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillType {
    Standard,
    Flow,
}

pub fn discover_skills(extra_dirs: &[PathBuf]) -> Result<HashMap<String, Skill>> {
    let mut skills = HashMap::new();

    // Builtin skills
    if let Ok(dir) = std::env::current_exe() {
        let builtin = dir.parent().unwrap_or(Path::new(".")).join("skills");
        load_skills_from_dir(&builtin, &mut skills)?;
    }

    // User skills
    if let Some(user_dir) = dirs::home_dir() {
        for subdir in [
            ".mekai/skills",
            ".claude/skills",
            ".codex/skills",
            ".agents/skills",
        ] {
            load_skills_from_dir(&user_dir.join(subdir), &mut skills)?;
        }
    }

    // Project skills (current dir)
    for subdir in [
        ".mekai/skills",
        ".claude/skills",
        ".codex/skills",
        ".agents/skills",
    ] {
        load_skills_from_dir(&PathBuf::from(subdir), &mut skills)?;
    }

    // Extra dirs
    for dir in extra_dirs {
        load_skills_from_dir(dir, &mut skills)?;
    }

    Ok(skills)
}

fn load_skills_from_dir(dir: &Path, skills: &mut HashMap<String, Skill>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(dir).max_depth(2) {
        let entry = entry?;
        let path = entry.path();
        if path.file_name() == Some(std::ffi::OsStr::new("SKILL.md")) {
            let content = std::fs::read_to_string(path)?;
            let parent = path.parent().unwrap_or(dir);
            let name = parent
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            let skill = Skill {
                name: name.clone(),
                description: extract_description(&content),
                content,
                skill_type: SkillType::Standard,
                source: path.to_path_buf(),
            };
            skills.insert(name, skill);
        }
    }

    Ok(())
}

fn extract_description(content: &str) -> String {
    content
        .lines()
        .next()
        .unwrap_or("")
        .trim_start_matches('#')
        .trim()
        .to_string()
}
