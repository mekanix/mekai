use std::collections::HashMap;
use std::sync::Arc;

use crate::kimi::config::Config;
use crate::kimi::error::Result;
use crate::kimi::llm::ChatProvider;
use crate::kimi::session::Session;
use crate::kimi::skill::Skill;

pub struct Runtime {
    pub config: Config,
    pub llm: Arc<dyn ChatProvider>,
    pub session: Session,
    pub builtin_args: BuiltinSystemPromptArgs,
    pub skills: HashMap<String, Skill>,
    pub role: String,
}

#[derive(Debug, Clone, Default)]
pub struct BuiltinSystemPromptArgs {
    pub now: String,
    pub work_dir: String,
    pub agents_md: String,
    pub skills: String,
}

impl Runtime {
    pub async fn create(
        config: Config,
        llm: Arc<dyn ChatProvider>,
        session: Session,
    ) -> Result<Self> {
        Ok(Self {
            config,
            llm,
            session,
            builtin_args: BuiltinSystemPromptArgs::default(),
            skills: HashMap::new(),
            role: "root".to_string(),
        })
    }
}
