use std::process::Stdio;

use crate::kimi::config::HookDef;
use crate::kimi::error::Result;

pub struct HookEngine {
    hooks: Vec<HookDef>,
}

impl HookEngine {
    pub fn new(hooks: Vec<HookDef>) -> Self {
        Self { hooks }
    }

    pub async fn trigger(
        &self,
        event: &str,
        matcher_value: &str,
        input_data: serde_json::Value,
    ) -> Result<Vec<serde_json::Value>> {
        let mut results = vec![];
        for hook in &self.hooks {
            if hook.event != event {
                continue;
            }
            if let Some(ref matcher) = hook.matcher
                && matcher != matcher_value
            {
                continue;
            }
            let output = run_hook_command(hook, &input_data).await?;
            results.push(output);
        }
        Ok(results)
    }
}

async fn run_hook_command(hook: &HookDef, input: &serde_json::Value) -> Result<serde_json::Value> {
    let mut child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&hook.command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(stdin) = child.stdin.take() {
        let input = input.to_string();
        tokio::spawn(async move {
            let mut stdin = stdin;
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stdin, input.as_bytes()).await;
        });
    }

    let timeout = hook.timeout.unwrap_or(30);
    let result = tokio::time::timeout(
        tokio::time::Duration::from_secs(timeout),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| {
        crate::kimi::error::MekaiError::Hook(format!("Hook timed out: {}", hook.command))
    })?;

    let output = result?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    match serde_json::from_str(&stdout) {
        Ok(val) => Ok(val),
        Err(_) => Ok(serde_json::json!({ "output": stdout.to_string() })),
    }
}
