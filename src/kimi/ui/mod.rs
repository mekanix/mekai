pub mod print;
pub mod shell;
pub mod vis;

use crate::kimi::error::Result;
use crate::kimi::session::Session;
use crate::kimi::soul::KimiSoul;

#[async_trait::async_trait]
pub trait Ui: Send + Sync {
    async fn run(
        &mut self,
        soul: &mut KimiSoul,
        session: &mut Session,
        prompt: Option<String>,
    ) -> Result<i32>;
}
