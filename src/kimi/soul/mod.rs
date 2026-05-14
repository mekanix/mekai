pub mod agent;
pub mod compaction;
pub mod context;
pub mod tools;

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use futures::{Stream, StreamExt};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, warn};

use crate::kimi::approval::ApprovalRuntime;
use crate::kimi::cancel::CancelToken;
use crate::kimi::config::LoopControl;
use crate::kimi::denwa_renji::DenwaRenji;
use crate::kimi::error::{MekaiError, Result};
use crate::kimi::hooks::HookEngine;
use crate::kimi::llm::{ChatProvider, Message, ToolCall, ToolDef};
use crate::kimi::notifications::NotificationManager;
use crate::kimi::session::Session;
use crate::kimi::soul::compaction::CompactionStrategy;
use crate::kimi::soul::context::Context;
use crate::kimi::soul::tools::Tool;
use crate::kimi::subagents::store::SubagentStore;
use crate::kimi::wire::types::WireEvent;

pub enum SoulEvent {
    Token(String),
    ToolCall(ToolCall),
    ToolResult {
        call_id: String,
        output: String,
    },
    Plan(String),
    Done(String),
    Error(String),
    ApprovalNeeded {
        tool_name: String,
        args: HashMap<String, serde_json::Value>,
    },
}

pub struct KimiSoul {
    pub agent: Agent,
    pub wire_tx: broadcast::Sender<WireEvent>,
    pub context: Context,
    pub tools: HashMap<String, Arc<dyn Tool>>,
    pub approval: Arc<ApprovalRuntime>,
    pub hooks: Option<HookEngine>,
    pub loop_control: LoopControl,
    pub compaction: Option<Box<dyn CompactionStrategy>>,
    pub plan_mode: bool,
    pub thinking: bool,
    pub session: Option<Session>,
    pub cancel: Arc<CancelToken>,
    pub btw_tx: mpsc::Sender<String>,
    pub btw_rx: mpsc::Receiver<String>,
    pub pending_plan: Option<Vec<ToolCall>>,
    pub checkpoints: DenwaRenji,
    pub notifications: NotificationManager,
    pub subagent_store: Arc<tokio::sync::RwLock<SubagentStore>>,
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub name: String,
    pub system_prompt: String,
    pub tools: Vec<ToolDef>,
}

impl KimiSoul {
    pub fn new(
        agent: Agent,
        wire_tx: broadcast::Sender<WireEvent>,
        approval: Arc<ApprovalRuntime>,
        max_context_size: usize,
        loop_control: LoopControl,
    ) -> Self {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        for tool in tools::builtin_tools() {
            tools.insert(tool.name().to_string(), tool);
        }

        let mut context = Context::new(max_context_size, loop_control.reserved_context_size);
        if !agent.system_prompt.is_empty() {
            context.push(Message::system(&agent.system_prompt));
        }

        let (btw_tx, btw_rx) = mpsc::channel(10);

        Self {
            agent,
            wire_tx,
            context,
            tools,
            approval,
            hooks: None,
            loop_control,
            compaction: Some(Box::new(compaction::SimpleCompaction::new())),
            plan_mode: false,
            thinking: false,
            session: None,
            cancel: Arc::new(CancelToken::new()),
            btw_tx,
            btw_rx,
            pending_plan: None,
            checkpoints: DenwaRenji::new(10),
            notifications: NotificationManager::new(100),
            subagent_store: crate::kimi::soul::tools::SUBAGENT_STORE
                .get()
                .cloned()
                .unwrap_or_else(|| Arc::new(tokio::sync::RwLock::new(SubagentStore::new()))),
        }
    }

    pub fn with_session(mut self, session: Session) -> Self {
        self.session = Some(session);
        self
    }

    pub fn with_plan_mode(mut self, plan: bool) -> Self {
        self.plan_mode = plan;
        self
    }

    pub fn with_thinking(mut self, thinking: bool) -> Self {
        self.thinking = thinking;
        if thinking && !self.context.messages.is_empty() {
            self.context.push(Message::system(
                "Think step by step and explain your reasoning before giving the final answer.",
            ));
        }
        self
    }

    pub fn with_hooks(mut self, hooks: HookEngine) -> Self {
        self.hooks = Some(hooks);
        self
    }

    pub fn with_loop_control(mut self, loop_control: LoopControl) -> Self {
        self.loop_control = loop_control;
        self
    }

    pub fn with_context_messages(mut self, messages: Vec<Message>) -> Self {
        for msg in messages {
            self.context.push(msg);
        }
        self
    }

    pub fn with_cancel(mut self, cancel: Arc<CancelToken>) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn with_compaction(mut self, compaction: Box<dyn CompactionStrategy>) -> Self {
        self.compaction = Some(compaction);
        self
    }

    pub fn checkpoint(&mut self, description: impl Into<String>) -> String {
        let id = self
            .checkpoints
            .checkpoint(self.context.messages.clone(), description);
        let _notification_fut = self
            .notifications
            .push(crate::kimi::notifications::Notification {
                id: id.clone(),
                level: crate::kimi::notifications::NotificationLevel::Success,
                message: format!("Checkpoint created: {id}"),
                source: "denwa_renji".to_string(),
            });
        self.emit(WireEvent::Notification {
            level: "success".to_string(),
            message: format!("Checkpoint created: {id}"),
            timestamp: chrono::Utc::now(),
        });
        id
    }

    pub fn rollback(&mut self, id: &str) -> bool {
        if let Some(messages) = self.checkpoints.rollback(id) {
            self.context.messages = messages;
            let _notification_fut =
                self.notifications
                    .push(crate::kimi::notifications::Notification {
                        id: id.to_string(),
                        level: crate::kimi::notifications::NotificationLevel::Success,
                        message: format!("Rolled back to checkpoint: {id}"),
                        source: "denwa_renji".to_string(),
                    });
            self.emit(WireEvent::Notification {
                level: "success".to_string(),
                message: format!("Rolled back to checkpoint: {id}"),
                timestamp: chrono::Utc::now(),
            });
            true
        } else {
            false
        }
    }

    pub fn list_checkpoints(&self) -> Vec<&crate::kimi::denwa_renji::Checkpoint> {
        self.checkpoints.list()
    }

    fn archive_turn(&mut self) {
        if let Some(ref mut session) = self.session {
            let turn = session.state.archive.len();
            let summary = self
                .context
                .messages
                .iter()
                .rev()
                .take(3)
                .map(|m| format!("[{}] {}", m.role, &m.content[..80.min(m.content.len())]))
                .collect::<Vec<_>>()
                .join(" | ");
            session
                .state
                .archive
                .push(crate::kimi::session_state::ArchivedTurn { turn, summary });
            let _ = session.save_state();
        }
    }

    pub fn get_btw_sender(&self) -> mpsc::Sender<String> {
        self.btw_tx.clone()
    }

    pub async fn run(&mut self, input: &str, llm: Arc<dyn ChatProvider>) -> Result<String> {
        // If there's a pending plan, execute it instead of starting a new turn
        if let Some(plan) = self.pending_plan.take() {
            self.emit(WireEvent::TurnBegin {
                turn: self.context.len(),
                timestamp: chrono::Utc::now(),
            });
            let result = self.execute_plan(plan, llm.clone()).await;
            self.emit(WireEvent::TurnEnd {
                turn: self.context.len(),
                timestamp: chrono::Utc::now(),
            });
            return result;
        }

        self.emit(WireEvent::TurnBegin {
            turn: self.context.len(),
            timestamp: chrono::Utc::now(),
        });

        if let Some(ref hooks) = self.hooks {
            let _ = hooks
                .trigger("UserPromptSubmit", "", serde_json::json!({"prompt": input}))
                .await;
        }

        let user_msg = Message::user(input);
        self.push_context(user_msg)?;

        let mut responses = Vec::new();
        let first = self.agent_loop(llm.clone()).await;
        match first {
            Ok(r) => responses.push(r),
            Err(e) => {
                self.emit(WireEvent::TurnEnd {
                    turn: self.context.len(),
                    timestamp: chrono::Utc::now(),
                });
                return Err(e);
            }
        }

        for i in 1..=self.loop_control.max_ralph_iterations {
            if self.cancel.is_cancelled() {
                break;
            }
            self.emit(WireEvent::RalphTurnBegin {
                iteration: i as usize,
                timestamp: chrono::Utc::now(),
            });
            let _ = self.push_context(Message::user("Continue."));
            match self.agent_loop(llm.clone()).await {
                Ok(r) => responses.push(r),
                Err(_) => break,
            }
            self.emit(WireEvent::RalphTurnEnd {
                iteration: i as usize,
                timestamp: chrono::Utc::now(),
            });
        }

        self.emit(WireEvent::TurnEnd {
            turn: self.context.len(),
            timestamp: chrono::Utc::now(),
        });

        if let Some(ref mut session) = self.session {
            if session.title.starts_with("Session ") {
                session.generate_title(input);
                let _ = session.save();
            }
            session.touch();
            let _ = session.save();
        }

        Ok(responses.join("\n\n"))
    }

    async fn agent_loop(&mut self, llm: Arc<dyn ChatProvider>) -> Result<String> {
        let max_steps = self.loop_control.max_steps_per_turn;
        let max_retries = self.loop_control.max_retries_per_step;

        for step in 0..max_steps {
            if self.cancel.is_cancelled() {
                return Ok("[Cancelled]".to_string());
            }

            // Check for BTW messages
            while let Ok(btw) = self.btw_rx.try_recv() {
                self.emit(WireEvent::BtwBegin {
                    question: btw.clone(),
                    timestamp: chrono::Utc::now(),
                });
                self.push_context(Message::user(format!("[BTW] {btw}")))?;
                self.emit(WireEvent::BtwEnd {
                    answer: String::new(),
                    timestamp: chrono::Utc::now(),
                });
            }

            self.emit(WireEvent::StepBegin {
                step,
                timestamp: chrono::Utc::now(),
            });

            if let Some(ref compaction) = self.compaction
                && self
                    .context
                    .needs_compaction(self.loop_control.compaction_trigger_ratio)
            {
                self.emit(WireEvent::CompactionBegin {
                    timestamp: chrono::Utc::now(),
                });
                let compacted = compaction.compact(&self.context.messages).await?;
                self.context.messages = compacted;
                self.emit(WireEvent::CompactionEnd {
                    timestamp: chrono::Utc::now(),
                });
                self.archive_turn();
            }

            let tool_defs = if self.agent.tools.is_empty() {
                let defs: Vec<ToolDef> = self
                    .tools
                    .values()
                    .map(|t| ToolDef {
                        name: t.name().to_string(),
                        description: t.description().to_string(),
                        parameters: t.parameters(),
                    })
                    .collect();
                Some(defs)
            } else {
                Some(self.agent.tools.clone())
            };

            let response = match llm.chat(self.context.messages.clone(), tool_defs).await {
                Ok(r) => r,
                Err(e) => {
                    warn!("LLM error at step {}: {}", step, e);
                    if step < max_retries {
                        continue;
                    }
                    self.emit(WireEvent::StepInterrupted {
                        step,
                        reason: e.to_string(),
                        timestamp: chrono::Utc::now(),
                    });
                    return Err(e);
                }
            };

            let message = response.message;

            if !message.content.is_empty() && message.tool_calls.is_none() {
                let assistant_msg = Message::assistant(&message.content);
                self.push_context(assistant_msg)?;
                return Ok(message.content);
            }

            if let Some(ref calls) = message.tool_calls {
                let assistant_msg = Message {
                    role: "assistant".to_string(),
                    content: message.content.clone(),
                    tool_calls: Some(calls.clone()),
                    tool_call_id: None,
                };
                self.push_context(assistant_msg)?;

                // Plan mode: store plan and return it instead of executing
                if self.plan_mode {
                    let plan = self.format_plan(calls);
                    self.pending_plan = Some(calls.clone());
                    let plan_text = format!(
                        "{plan}\n\n[Plan mode: Use /execute to run this plan, or /plan to disable plan mode]"
                    );
                    self.emit(WireEvent::PlanDisplay {
                        plan: plan_text.clone(),
                        timestamp: chrono::Utc::now(),
                    });
                    return Ok(plan_text);
                }

                // Execute tools inline and continue loop
                for call in calls {
                    if self.cancel.is_cancelled() {
                        return Ok("[Cancelled]".to_string());
                    }
                    self.emit(WireEvent::ToolCall {
                        call: call.clone(),
                        timestamp: chrono::Utc::now(),
                    });
                    let result = self.execute_tool_call(call).await?;
                    self.emit(WireEvent::ToolResult {
                        call_id: call.id.clone(),
                        result: result.clone(),
                        timestamp: chrono::Utc::now(),
                    });
                    self.push_context(Message::tool(&result.output, &call.id))?;
                }
                continue;
            } else {
                let assistant_msg = Message::assistant(&message.content);
                self.push_context(assistant_msg)?;
                return Ok(message.content);
            }
        }

        warn!("Reached max_steps_per_turn ({})", max_steps);
        Ok("[Reached maximum steps per turn]".to_string())
    }

    async fn execute_plan(
        &mut self,
        calls: Vec<ToolCall>,
        _llm: Arc<dyn ChatProvider>,
    ) -> Result<String> {
        for call in &calls {
            if self.cancel.is_cancelled() {
                return Ok("[Cancelled]".to_string());
            }

            self.emit(WireEvent::ToolCall {
                call: call.clone(),
                timestamp: chrono::Utc::now(),
            });

            let result = self.execute_tool_call(call).await?;

            self.emit(WireEvent::ToolResult {
                call_id: call.id.clone(),
                result: result.clone(),
                timestamp: chrono::Utc::now(),
            });

            self.push_context(Message::tool(&result.output, &call.id))?;
        }

        Ok("Plan executed. Continuing...".to_string())
    }

    fn format_plan(&self, calls: &[ToolCall]) -> String {
        let mut plan = String::from("Plan:\n");
        for (i, call) in calls.iter().enumerate() {
            plan.push_str(&format!(
                "  {}. {}({})\n",
                i + 1,
                call.function.name,
                call.function.arguments
            ));
        }
        plan
    }

    async fn execute_tool_call(&self, call: &ToolCall) -> Result<tools::ToolResult> {
        let tool_name = &call.function.name;
        let args = parse_arguments(&call.function.arguments)?;

        debug!("Executing tool {} with args {:?}", tool_name, args);

        let tool = self
            .tools
            .get(tool_name)
            .ok_or_else(|| MekaiError::Tool(format!("Unknown tool: {tool_name}")))?;

        let request_id = uuid::Uuid::new_v4().to_string();
        self.emit(WireEvent::ApprovalRequest {
            request_id: request_id.clone(),
            tool_name: tool_name.clone(),
            arguments: args.clone(),
            timestamp: chrono::Utc::now(),
        });
        let approval_resp = self
            .approval
            .request_approval(tool_name.clone(), args.clone())
            .await?;
        self.emit(WireEvent::ApprovalResponse {
            request_id,
            approved: approval_resp.approved,
            timestamp: chrono::Utc::now(),
        });
        if !approval_resp.approved {
            return Ok(tools::ToolResult {
                success: false,
                output: format!("Approval denied for tool: {tool_name}"),
            });
        }

        match tool.execute(args).await {
            Ok(output) => Ok(tools::ToolResult {
                success: true,
                output,
            }),
            Err(e) => Ok(tools::ToolResult {
                success: false,
                output: format!("Tool error: {e}"),
            }),
        }
    }

    pub async fn run_with_events(
        &mut self,
        input: &str,
        llm: Arc<dyn ChatProvider>,
        event_tx: mpsc::Sender<SoulEvent>,
    ) -> Result<String> {
        // If there's a pending plan, execute it instead of starting a new turn
        if let Some(plan) = self.pending_plan.take() {
            self.emit(WireEvent::TurnBegin {
                turn: self.context.len(),
                timestamp: chrono::Utc::now(),
            });
            for call in &plan {
                if self.cancel.is_cancelled() {
                    let _ = event_tx
                        .send(SoulEvent::Done("[Cancelled]".to_string()))
                        .await;
                    self.emit(WireEvent::TurnEnd {
                        turn: self.context.len(),
                        timestamp: chrono::Utc::now(),
                    });
                    return Ok("[Cancelled]".to_string());
                }
                let _ = event_tx.send(SoulEvent::ToolCall(call.clone())).await;
                let result = self.execute_tool_call(call).await?;
                let _ = event_tx
                    .send(SoulEvent::ToolResult {
                        call_id: call.id.clone(),
                        output: result.output.clone(),
                    })
                    .await;
                self.push_context(Message::tool(&result.output, &call.id))?;
            }
            let _ = event_tx
                .send(SoulEvent::Done("Plan executed. Continuing...".to_string()))
                .await;
            self.emit(WireEvent::TurnEnd {
                turn: self.context.len(),
                timestamp: chrono::Utc::now(),
            });
            return Ok("Plan executed. Continuing...".to_string());
        }

        self.emit(WireEvent::TurnBegin {
            turn: self.context.len(),
            timestamp: chrono::Utc::now(),
        });

        if let Some(ref hooks) = self.hooks {
            let _ = hooks
                .trigger("UserPromptSubmit", "", serde_json::json!({"prompt": input}))
                .await;
        }

        let user_msg = Message::user(input);
        self.push_context(user_msg)?;

        let mut responses = Vec::new();
        let first = self.agent_loop_streaming(llm.clone(), &event_tx).await;
        match first {
            Ok(r) => responses.push(r),
            Err(e) => {
                self.emit(WireEvent::TurnEnd {
                    turn: self.context.len(),
                    timestamp: chrono::Utc::now(),
                });
                return Err(e);
            }
        }

        for i in 1..=self.loop_control.max_ralph_iterations {
            if self.cancel.is_cancelled() {
                break;
            }
            self.emit(WireEvent::RalphTurnBegin {
                iteration: i as usize,
                timestamp: chrono::Utc::now(),
            });
            let _ = event_tx
                .send(SoulEvent::Token(format!("\n[Ralph iteration {i}]\n")))
                .await;
            let _ = self.push_context(Message::user("Continue."));
            match self.agent_loop(llm.clone()).await {
                Ok(r) => responses.push(r),
                Err(_) => break,
            }
            self.emit(WireEvent::RalphTurnEnd {
                iteration: i as usize,
                timestamp: chrono::Utc::now(),
            });
        }

        self.emit(WireEvent::TurnEnd {
            turn: self.context.len(),
            timestamp: chrono::Utc::now(),
        });

        if let Some(ref mut session) = self.session {
            if session.title.starts_with("Session ") {
                session.generate_title(input);
                let _ = session.save();
            }
            session.touch();
            let _ = session.save();
        }

        Ok(responses.join("\n\n"))
    }

    async fn agent_loop_streaming(
        &mut self,
        llm: Arc<dyn ChatProvider>,
        event_tx: &mpsc::Sender<SoulEvent>,
    ) -> Result<String> {
        let max_steps = self.loop_control.max_steps_per_turn;
        let max_retries = self.loop_control.max_retries_per_step;

        for step in 0..max_steps {
            if self.cancel.is_cancelled() {
                let _ = event_tx
                    .send(SoulEvent::Done("[Cancelled]".to_string()))
                    .await;
                return Ok("[Cancelled]".to_string());
            }

            while let Ok(btw) = self.btw_rx.try_recv() {
                self.emit(WireEvent::BtwBegin {
                    question: btw.clone(),
                    timestamp: chrono::Utc::now(),
                });
                self.push_context(Message::user(format!("[BTW] {btw}")))?;
                self.emit(WireEvent::BtwEnd {
                    answer: String::new(),
                    timestamp: chrono::Utc::now(),
                });
            }

            self.emit(WireEvent::StepBegin {
                step,
                timestamp: chrono::Utc::now(),
            });

            if let Some(ref compaction) = self.compaction
                && self
                    .context
                    .needs_compaction(self.loop_control.compaction_trigger_ratio)
            {
                self.emit(WireEvent::CompactionBegin {
                    timestamp: chrono::Utc::now(),
                });
                let compacted = compaction.compact(&self.context.messages).await?;
                self.context.messages = compacted;
                self.emit(WireEvent::CompactionEnd {
                    timestamp: chrono::Utc::now(),
                });
                self.archive_turn();
            }

            let tool_defs = if self.agent.tools.is_empty() {
                let defs: Vec<ToolDef> = self
                    .tools
                    .values()
                    .map(|t| ToolDef {
                        name: t.name().to_string(),
                        description: t.description().to_string(),
                        parameters: t.parameters(),
                    })
                    .collect();
                Some(defs)
            } else {
                Some(self.agent.tools.clone())
            };

            let messages = self.context.messages.clone();

            tracing::info!(
                "agent_loop_streaming step {}: calling stream_chat with {} messages",
                step,
                messages.len()
            );
            let stream_result = llm.stream_chat(messages, tool_defs).await;
            let mut stream = match stream_result {
                Ok(s) => {
                    tracing::info!("agent_loop_streaming step {}: stream started", step);
                    s
                }
                Err(e) => {
                    warn!("LLM error at step {}: {}", step, e);
                    if step < max_retries {
                        continue;
                    }
                    self.emit(WireEvent::StepInterrupted {
                        step,
                        reason: e.to_string(),
                        timestamp: chrono::Utc::now(),
                    });
                    let _ = event_tx.send(SoulEvent::Error(e.to_string())).await;
                    return Err(e);
                }
            };

            let mut full_response = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();

            use futures::StreamExt;
            let mut chunk_count = 0;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => {
                        chunk_count += 1;
                        if !chunk.delta.is_empty() {
                            full_response.push_str(&chunk.delta);
                            let _ = event_tx.send(SoulEvent::Token(chunk.delta)).await;
                        }
                        if let Some(calls) = chunk.tool_calls {
                            tool_calls.extend(calls);
                        }
                        if let Some(reason) = chunk.finish_reason
                            && reason == "stop"
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.send(SoulEvent::Error(e.to_string())).await;
                        warn!("Stream error at step {}: {}", step, e);
                        if step < max_retries {
                            break;
                        }
                        self.emit(WireEvent::StepInterrupted {
                            step,
                            reason: e.to_string(),
                            timestamp: chrono::Utc::now(),
                        });
                        return Err(e);
                    }
                }
            }
            tracing::info!(
                "agent_loop_streaming step {}: stream ended, {} chunks, response len {}",
                step,
                chunk_count,
                full_response.len()
            );

            if !full_response.is_empty() && tool_calls.is_empty() {
                let assistant_msg = Message::assistant(&full_response);
                self.push_context(assistant_msg)?;
                let _ = event_tx.send(SoulEvent::Done(full_response.clone())).await;
                return Ok(full_response);
            }

            if !tool_calls.is_empty() {
                let assistant_msg = Message {
                    role: "assistant".to_string(),
                    content: full_response.clone(),
                    tool_calls: Some(tool_calls.clone()),
                    tool_call_id: None,
                };
                self.push_context(assistant_msg)?;

                if self.plan_mode {
                    let plan = self.format_plan(&tool_calls);
                    self.pending_plan = Some(tool_calls.clone());
                    let plan_text = format!(
                        "{plan}\n\n[Plan mode: Use /execute to run this plan, or /plan to disable plan mode]"
                    );
                    self.emit(WireEvent::PlanDisplay {
                        plan: plan_text.clone(),
                        timestamp: chrono::Utc::now(),
                    });
                    let _ = event_tx.send(SoulEvent::Plan(plan_text.clone())).await;
                    let _ = event_tx.send(SoulEvent::Done(plan_text.clone())).await;
                    return Ok(plan_text);
                }

                for call in &tool_calls {
                    if self.cancel.is_cancelled() {
                        let _ = event_tx
                            .send(SoulEvent::Done("[Cancelled]".to_string()))
                            .await;
                        return Ok("[Cancelled]".to_string());
                    }

                    self.emit(WireEvent::ToolCall {
                        call: call.clone(),
                        timestamp: chrono::Utc::now(),
                    });
                    let _ = event_tx.send(SoulEvent::ToolCall(call.clone())).await;

                    let result = self.execute_tool_call(call).await?;

                    self.emit(WireEvent::ToolResult {
                        call_id: call.id.clone(),
                        result: result.clone(),
                        timestamp: chrono::Utc::now(),
                    });
                    let _ = event_tx
                        .send(SoulEvent::ToolResult {
                            call_id: call.id.clone(),
                            output: result.output.clone(),
                        })
                        .await;

                    self.push_context(Message::tool(&result.output, &call.id))?;
                }
                continue;
            }

            let assistant_msg = Message::assistant(&full_response);
            self.push_context(assistant_msg)?;
            let _ = event_tx.send(SoulEvent::Done(full_response.clone())).await;
            return Ok(full_response);
        }

        warn!("Reached max_steps_per_turn ({})", max_steps);
        let _ = event_tx
            .send(SoulEvent::Done(
                "[Reached maximum steps per turn]".to_string(),
            ))
            .await;
        Ok("[Reached maximum steps per turn]".to_string())
    }

    pub fn run_streaming(
        &mut self,
        input: String,
        llm: Arc<dyn ChatProvider>,
    ) -> Pin<Box<dyn Stream<Item = SoulEvent> + Send>> {
        let user_msg = Message::user(&input);
        let _ = self.push_context(user_msg.clone());

        let tool_defs = if self.agent.tools.is_empty() {
            let defs: Vec<ToolDef> = self
                .tools
                .values()
                .map(|t| ToolDef {
                    name: t.name().to_string(),
                    description: t.description().to_string(),
                    parameters: t.parameters(),
                })
                .collect();
            Some(defs)
        } else {
            Some(self.agent.tools.clone())
        };

        let messages = self.context.messages.clone();
        let approval = Arc::clone(&self.approval);
        let tools: HashMap<String, Arc<dyn Tool>> = self.tools.clone();
        let plan_mode = self.plan_mode;

        let stream = async_stream::stream! {
            yield SoulEvent::Token("".to_string());

            match llm.stream_chat(messages, tool_defs).await {
                Ok(mut stream) => {
                    let mut full_response = String::new();
                    let mut tool_calls: Vec<ToolCall> = Vec::new();

                    while let Some(chunk) = stream.as_mut().next().await {
                        match chunk {
                            Ok(chunk) => {
                                if !chunk.delta.is_empty() {
                                    full_response.push_str(&chunk.delta);
                                    yield SoulEvent::Token(chunk.delta);
                                }
                                if let Some(calls) = chunk.tool_calls {
                                    tool_calls.extend(calls);
                                }
                            }
                            Err(e) => {
                                yield SoulEvent::Error(e.to_string());
                                return;
                            }
                        }
                    }

                    if !tool_calls.is_empty() {
                        if plan_mode {
                            let plan = tool_calls.iter().enumerate().map(|(i, call)| {
                                format!("  {}. {}({})", i + 1, call.function.name, call.function.arguments)
                            }).collect::<Vec<_>>().join("\n");
                            yield SoulEvent::Plan(format!("Plan:\n{plan}"));
                        } else {
                            for call in &tool_calls {
                                yield SoulEvent::ToolCall(call.clone());

                                let args = match parse_arguments(&call.function.arguments) {
                                    Ok(a) => a,
                                    Err(e) => {
                                        yield SoulEvent::Error(e.to_string());
                                        continue;
                                    }
                                };

                                let approved = match approval.request_approval(call.function.name.clone(), args.clone()).await {
                                    Ok(resp) => resp.approved,
                                    Err(_) => false,
                                };

                                if !approved {
                                    yield SoulEvent::ToolResult {
                                        call_id: call.id.clone(),
                                        output: format!("Approval denied for tool: {}", call.function.name),
                                    };
                                    continue;
                                }

                                if let Some(tool) = tools.get(&call.function.name) {
                                    match tool.execute(args).await {
                                        Ok(output) => {
                                            yield SoulEvent::ToolResult {
                                                call_id: call.id.clone(),
                                                output: output.clone(),
                                            };
                                        }
                                        Err(e) => {
                                            yield SoulEvent::ToolResult {
                                                call_id: call.id.clone(),
                                                output: format!("Tool error: {e}"),
                                            };
                                        }
                                    }
                                } else {
                                    yield SoulEvent::ToolResult {
                                        call_id: call.id.clone(),
                                        output: format!("Unknown tool: {}", call.function.name),
                                    };
                                }
                            }
                        }
                    }

                    yield SoulEvent::Done(full_response);
                }
                Err(e) => {
                    yield SoulEvent::Error(e.to_string());
                }
            }
        };

        Box::pin(stream)
    }

    fn push_context(&mut self, message: Message) -> Result<()> {
        self.context.push(message.clone());
        if let Some(ref session) = self.session {
            session.append_context(&message)?;
        }
        Ok(())
    }

    fn emit(&self, event: WireEvent) {
        let _ = self.wire_tx.send(event.clone());
        if let Some(ref session) = self.session {
            let _ = session.append_wire_event(&event);
        }
    }
}

fn parse_arguments(args: &str) -> Result<HashMap<String, serde_json::Value>> {
    if args.trim().is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(args).map_err(|e| MekaiError::Tool(format!("Invalid arguments JSON: {e}")))
}
