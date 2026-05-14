use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::{RwLock, mpsc};
use tracing::{error, info};

use crate::kimi::background::TaskStatus;
use crate::kimi::background::tasks::{BackgroundTask, TaskSpec};
use crate::kimi::error::Result;

#[derive(Clone)]
pub struct BackgroundTaskManager {
    inner: Arc<RwLock<ManagerInner>>,
}

struct ManagerInner {
    tasks: HashMap<String, BackgroundTask>,
    max_running: usize,
    agent_task_timeout: Duration,
    keep_alive: bool,
    completion_tx: mpsc::Sender<String>,
    running_children: HashMap<String, tokio::process::Child>,
}

impl BackgroundTaskManager {
    pub fn new(max_running: usize, agent_task_timeout: Duration, keep_alive: bool) -> Self {
        let (completion_tx, _completion_rx) = mpsc::channel(100);
        Self {
            inner: Arc::new(RwLock::new(ManagerInner {
                tasks: HashMap::new(),
                max_running,
                agent_task_timeout,
                keep_alive,
                completion_tx,
                running_children: HashMap::new(),
            })),
        }
    }

    pub async fn create_bash_task(&self, spec: TaskSpec) -> Result<String> {
        {
            let guard = self.inner.read().await;
            let running = guard
                .tasks
                .values()
                .filter(|t| t.status == TaskStatus::Running)
                .count();
            if running >= guard.max_running {
                return Err(crate::kimi::error::MekaiError::BackgroundTask(format!(
                    "Max running tasks ({}) reached. Try again later.",
                    guard.max_running
                )));
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        let task_dir = task_data_dir().join(&id);
        tokio::fs::create_dir_all(&task_dir).await?;

        let command = spec.command.clone().ok_or_else(|| {
            crate::kimi::error::MekaiError::BackgroundTask("No command specified".into())
        })?;

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&command);

        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }
        if let Some(env) = &spec.env {
            cmd.envs(env);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let child = cmd.spawn()?;
        let pid = child.id();

        let keep_alive = self.inner.read().await.keep_alive;
        let meta = serde_json::json!({
            "keep_alive": keep_alive,
        });
        let _ = tokio::fs::write(task_dir.join("meta.json"), meta.to_string()).await;

        let mut task = BackgroundTask::new_bash(id.clone(), spec.clone(), pid);
        task.status = TaskStatus::Running;

        {
            let mut guard = self.inner.write().await;
            guard.tasks.insert(id.clone(), task);
            guard.running_children.insert(id.clone(), child);
        }

        let task_id = id.clone();
        let inner = Arc::clone(&self.inner);
        let completion_tx = self.inner.read().await.completion_tx.clone();

        tokio::spawn(async move {
            let result = monitor_bash_task(&task_id, &spec, &task_dir, inner.clone()).await;
            let _ = completion_tx.send(task_id.clone()).await;
            match result {
                Ok(output) => {
                    info!("Background task {task_id} completed");
                    let _ = tokio::fs::write(task_dir.join("output.txt"), &output).await;
                    let _ = tokio::fs::write(task_dir.join("status.txt"), "completed").await;
                    let mut guard = inner.write().await;
                    if let Some(t) = guard.tasks.get_mut(&task_id) {
                        t.status = TaskStatus::Completed;
                        t.output = Some(output);
                        t.completed_at = Some(chrono::Utc::now());
                    }
                    guard.running_children.remove(&task_id);
                }
                Err(e) => {
                    error!("Background task {task_id} failed: {e}");
                    let _ = tokio::fs::write(task_dir.join("error.txt"), e.to_string()).await;
                    let _ = tokio::fs::write(task_dir.join("status.txt"), "failed").await;
                    let mut guard = inner.write().await;
                    if let Some(t) = guard.tasks.get_mut(&task_id) {
                        t.status = TaskStatus::Failed;
                        t.error = Some(e.to_string());
                        t.completed_at = Some(chrono::Utc::now());
                    }
                    guard.running_children.remove(&task_id);
                }
            }
        });

        Ok(id)
    }

    pub async fn create_agent_task(&self, spec: TaskSpec) -> Result<String> {
        {
            let guard = self.inner.read().await;
            let running = guard
                .tasks
                .values()
                .filter(|t| t.status == TaskStatus::Running)
                .count();
            if running >= guard.max_running {
                return Err(crate::kimi::error::MekaiError::BackgroundTask(format!(
                    "Max running tasks ({}) reached. Try again later.",
                    guard.max_running
                )));
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        let task_dir = task_data_dir().join(&id);
        tokio::fs::create_dir_all(&task_dir).await?;

        let prompt = spec.command.clone().unwrap_or_default();
        let mut task = BackgroundTask::new_agent(id.clone());
        task.status = TaskStatus::Running;

        {
            let mut guard = self.inner.write().await;
            guard.tasks.insert(id.clone(), task);
        }

        let timeout = self.inner.read().await.agent_task_timeout;
        let keep_alive = self.inner.read().await.keep_alive;

        let meta = serde_json::json!({
            "keep_alive": keep_alive,
            "timeout_secs": timeout.as_secs(),
        });
        let _ = tokio::fs::write(task_dir.join("meta.json"), meta.to_string()).await;

        let task_id = id.clone();
        let inner = Arc::clone(&self.inner);
        let completion_tx = self.inner.read().await.completion_tx.clone();

        tokio::spawn(async move {
            let result = tokio::time::timeout(timeout, run_agent_task(&prompt)).await;
            let _ = completion_tx.send(task_id.clone()).await;
            let mut guard = inner.write().await;
            if let Some(t) = guard.tasks.get_mut(&task_id) {
                match result {
                    Ok(Ok(output)) => {
                        t.status = TaskStatus::Completed;
                        t.output = Some(output);
                        t.completed_at = Some(chrono::Utc::now());
                        let _ = tokio::fs::write(
                            task_dir.join("output.txt"),
                            t.output.as_deref().unwrap_or(""),
                        )
                        .await;
                        let _ = tokio::fs::write(task_dir.join("status.txt"), "completed").await;
                    }
                    Ok(Err(e)) => {
                        t.status = TaskStatus::Failed;
                        t.error = Some(e.to_string());
                        t.completed_at = Some(chrono::Utc::now());
                        let _ = tokio::fs::write(task_dir.join("error.txt"), e.to_string()).await;
                        let _ = tokio::fs::write(task_dir.join("status.txt"), "failed").await;
                    }
                    Err(_) => {
                        t.status = TaskStatus::Failed;
                        t.error = Some("Agent task timed out".to_string());
                        t.completed_at = Some(chrono::Utc::now());
                        let _ =
                            tokio::fs::write(task_dir.join("error.txt"), "Agent task timed out")
                                .await;
                        let _ = tokio::fs::write(task_dir.join("status.txt"), "failed").await;
                    }
                }
            }
        });

        Ok(id)
    }

    pub async fn list_tasks(&self) -> Vec<BackgroundTask> {
        self.inner.read().await.tasks.values().cloned().collect()
    }

    pub async fn get_task(&self, id: &str) -> Option<BackgroundTask> {
        self.inner.read().await.tasks.get(id).cloned()
    }

    pub async fn kill(&self, id: &str) -> Result<()> {
        let mut guard = self.inner.write().await;
        if let Some(child) = guard.running_children.get_mut(id) {
            let _ = child.kill().await;
        }
        if let Some(task) = guard.tasks.get_mut(id) {
            task.status = TaskStatus::Cancelled;
            task.completed_at = Some(chrono::Utc::now());
        }
        guard.running_children.remove(id);
        Ok(())
    }

    pub async fn wait(&self, id: &str) -> Result<()> {
        loop {
            let guard = self.inner.read().await;
            if let Some(task) = guard.tasks.get(id) {
                match task.status {
                    TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => break,
                    _ => {}
                }
            } else {
                return Err(crate::kimi::error::MekaiError::BackgroundTask(format!(
                    "Task {id} not found"
                )));
            }
            drop(guard);
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        Ok(())
    }

    pub async fn recover(&self) -> Result<()> {
        let dir = task_data_dir();
        if !dir.exists() {
            return Ok(());
        }

        let mut guard = self.inner.write().await;
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let task_dir = entry.path();
            let id = task_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            if id.is_empty() || guard.tasks.contains_key(&id) {
                continue;
            }

            let meta_file = task_dir.join("meta.json");
            let keep_alive = if meta_file.exists() {
                std::fs::read_to_string(&meta_file)
                    .ok()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|v| v.get("keep_alive").and_then(|k| k.as_bool()))
                    .unwrap_or(false)
            } else {
                false
            };

            let status_file = task_dir.join("status.txt");
            let status = if status_file.exists() {
                std::fs::read_to_string(&status_file).ok()
            } else {
                None
            };

            let task_status = match status.as_deref() {
                Some("completed") => TaskStatus::Completed,
                Some("failed") => TaskStatus::Failed,
                Some("cancelled") => TaskStatus::Cancelled,
                None if keep_alive => TaskStatus::Running,
                _ => TaskStatus::Lost,
            };

            let output = if task_dir.join("output.txt").exists() {
                std::fs::read_to_string(task_dir.join("output.txt")).ok()
            } else {
                None
            };

            let error = if task_dir.join("error.txt").exists() {
                std::fs::read_to_string(task_dir.join("error.txt")).ok()
            } else {
                None
            };

            let task = BackgroundTask {
                id: id.clone(),
                status: task_status,
                spec: TaskSpec {
                    command: None,
                    cwd: None,
                    env: None,
                    timeout_secs: None,
                },
                created_at: chrono::Utc::now(),
                completed_at: Some(chrono::Utc::now()),
                output,
                error,
                pid: None,
            };
            guard.tasks.insert(id, task);
        }
        Ok(())
    }

    pub async fn reconcile(&self) -> Result<()> {
        let mut guard = self.inner.write().await;
        for (id, task) in guard.tasks.iter_mut() {
            if task.status == TaskStatus::Running {
                continue;
            }
            let task_dir = task_data_dir().join(id);
            if let Ok(status) = tokio::fs::read_to_string(task_dir.join("status.txt")).await {
                let new_status = match status.trim() {
                    "completed" => TaskStatus::Completed,
                    "failed" => TaskStatus::Failed,
                    _ => continue,
                };
                if task.status != new_status {
                    task.status = new_status;
                    if let Ok(output) = tokio::fs::read_to_string(task_dir.join("output.txt")).await
                    {
                        task.output = Some(output);
                    }
                    if let Ok(err) = tokio::fs::read_to_string(task_dir.join("error.txt")).await {
                        task.error = Some(err);
                    }
                }
            }
        }
        Ok(())
    }
}

async fn monitor_bash_task(
    _task_id: &str,
    spec: &TaskSpec,
    task_dir: &Path,
    _inner: Arc<RwLock<ManagerInner>>,
) -> Result<String> {
    let command = spec.command.clone().ok_or_else(|| {
        crate::kimi::error::MekaiError::BackgroundTask("No command specified".into())
    })?;

    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(&command);

    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    if let Some(env) = &spec.env {
        cmd.envs(env);
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn()?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let mut stdout_reader = tokio::io::BufReader::new(stdout).lines();
    let mut stderr_reader = tokio::io::BufReader::new(stderr).lines();

    let mut output = String::new();
    let out_file = task_dir.join("output.txt");
    let mut file = tokio::fs::File::create(&out_file).await?;

    let timeout = spec.timeout_secs.unwrap_or(300);
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(timeout), async {
        loop {
            tokio::select! {
                line = stdout_reader.next_line() => {
                    if let Ok(Some(line)) = line {
                        output.push_str(&line);
                        output.push('\n');
                        let _ = file.write_all(line.as_bytes()).await;
                        let _ = file.write_all(b"\n").await;
                    }
                }
                line = stderr_reader.next_line() => {
                    if let Ok(Some(line)) = line {
                        output.push_str("[stderr] ");
                        output.push_str(&line);
                        output.push('\n');
                    }
                }
                status = child.wait() => {
                    let _ = status?;
                    break;
                }
            }
        }
        Ok::<(), crate::kimi::error::MekaiError>(())
    })
    .await;

    match result {
        Ok(Ok(())) => Ok(output),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            let _ = child.kill().await;
            Err(crate::kimi::error::MekaiError::BackgroundTask(
                "Task timed out".into(),
            ))
        }
    }
}

async fn run_agent_task(prompt: &str) -> Result<String> {
    if prompt.is_empty() {
        return Ok("Agent task completed with no prompt.".to_string());
    }

    let llm = crate::kimi::soul::tools::LLM_PROVIDER
        .get()
        .cloned()
        .ok_or_else(|| crate::kimi::error::MekaiError::Tool("LLM provider not available".into()))?;

    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let approval = Arc::new(crate::kimi::approval::ApprovalRuntime::new());
    approval.set_yolo(true).await;

    let tool_defs: Vec<crate::kimi::llm::ToolDef> = crate::kimi::soul::tools::builtin_tools()
        .into_iter()
        .map(|t| crate::kimi::llm::ToolDef {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters(),
        })
        .collect();

    let agent = crate::kimi::soul::Agent {
        name: "background_agent".to_string(),
        system_prompt: "You are a background agent. Work independently and return concise results."
            .to_string(),
        tools: tool_defs,
    };

    let mut soul = crate::kimi::soul::KimiSoul::new(
        agent,
        tx,
        approval,
        50000,
        crate::kimi::config::LoopControl::default(),
    );
    match soul.run(prompt, llm).await {
        Ok(result) => Ok(result),
        Err(e) => Err(e),
    }
}

fn task_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mekai")
        .join("background_tasks")
}
