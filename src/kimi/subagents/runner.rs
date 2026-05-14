use std::sync::Arc;

use crate::kimi::error::Result;
use crate::kimi::llm::ChatProvider;
use crate::kimi::session::Session;
use crate::kimi::soul::Agent;
use crate::kimi::soul::tools::builtin_tools;
use crate::kimi::subagents::SubagentType;

pub struct SubagentRunner;

impl SubagentRunner {
    pub async fn run_foreground(
        subagent_type: &SubagentType,
        task: &str,
        llm: Arc<dyn ChatProvider>,
        _session: &Session,
    ) -> Result<String> {
        let llm_ref = crate::kimi::soul::tools::LLM_PROVIDER
            .get()
            .cloned()
            .unwrap_or(llm);

        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let approval = Arc::new(crate::kimi::approval::ApprovalRuntime::new());
        approval.set_yolo(true).await;

        let subagent_tools: Vec<crate::kimi::llm::ToolDef> = builtin_tools()
            .into_iter()
            .filter(|t| subagent_type.tools.contains(&t.name().to_string()))
            .map(|t| crate::kimi::llm::ToolDef {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
            })
            .collect();

        let agent = Agent {
            name: subagent_type.name.clone(),
            system_prompt: format!(
                "{system}\n\nYou are a specialized subagent. Your task: {task}\n\nBe concise. Focus only on the assigned task.",
                system = subagent_type.system_prompt
            ),
            tools: subagent_tools,
        };

        let mut soul = crate::kimi::soul::KimiSoul::new(
            agent,
            tx,
            approval,
            100000,
            crate::kimi::config::LoopControl::default(),
        );
        let id = uuid::Uuid::new_v4().to_string();
        if let Some(store) = crate::kimi::soul::tools::SUBAGENT_STORE.get() {
            store
                .write()
                .await
                .insert(crate::kimi::subagents::store::SubagentRecord {
                    id: id.clone(),
                    agent_type: subagent_type.name.clone(),
                    status: "running".to_string(),
                });
        }

        match soul.run(task, Arc::clone(&llm_ref)).await {
            Ok(result) => {
                if let Some(store) = crate::kimi::soul::tools::SUBAGENT_STORE.get() {
                    store
                        .write()
                        .await
                        .insert(crate::kimi::subagents::store::SubagentRecord {
                            id: id.clone(),
                            agent_type: subagent_type.name.clone(),
                            status: "completed".to_string(),
                        });
                }
                Ok(format!(
                    "Subagent '{}' completed task:\n---\n{task}\n---\nResult:\n{result}",
                    subagent_type.name
                ))
            }
            Err(e) => {
                if let Some(store) = crate::kimi::soul::tools::SUBAGENT_STORE.get() {
                    store
                        .write()
                        .await
                        .insert(crate::kimi::subagents::store::SubagentRecord {
                            id: id.clone(),
                            agent_type: subagent_type.name.clone(),
                            status: "failed".to_string(),
                        });
                }
                Ok(format!("Subagent '{}' failed: {e}", subagent_type.name))
            }
        }
    }

    pub async fn run_background(
        subagent_type: &SubagentType,
        task: &str,
        llm: Arc<dyn ChatProvider>,
        _session: &Session,
    ) -> Result<String> {
        let llm_ref = crate::kimi::soul::tools::LLM_PROVIDER
            .get()
            .cloned()
            .unwrap_or(llm);

        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let approval = Arc::new(crate::kimi::approval::ApprovalRuntime::new());
        approval.set_yolo(true).await;

        let subagent_tools: Vec<crate::kimi::llm::ToolDef> = builtin_tools()
            .into_iter()
            .filter(|t| subagent_type.tools.contains(&t.name().to_string()))
            .map(|t| crate::kimi::llm::ToolDef {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
            })
            .collect();

        let agent = Agent {
            name: subagent_type.name.clone(),
            system_prompt: format!(
                "{system}\n\nYou are a specialized subagent. Your task: {task}\n\nBe concise. Focus only on the assigned task.",
                system = subagent_type.system_prompt
            ),
            tools: subagent_tools,
        };

        let mut soul = crate::kimi::soul::KimiSoul::new(
            agent,
            tx,
            approval,
            100000,
            crate::kimi::config::LoopControl::default(),
        );
        let task = task.to_string();
        let name = subagent_type.name.clone();
        let task2 = task.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let id2 = id.clone();
        if let Some(store) = crate::kimi::soul::tools::SUBAGENT_STORE.get() {
            store
                .write()
                .await
                .insert(crate::kimi::subagents::store::SubagentRecord {
                    id: id.clone(),
                    agent_type: name.clone(),
                    status: "running".to_string(),
                });
        }
        tokio::spawn(async move {
            match soul.run(&task2, llm_ref).await {
                Ok(result) => {
                    tracing::info!("Background subagent {name} completed: {result}");
                    if let Some(store) = crate::kimi::soul::tools::SUBAGENT_STORE.get() {
                        store
                            .write()
                            .await
                            .insert(crate::kimi::subagents::store::SubagentRecord {
                                id: id2.clone(),
                                agent_type: name.clone(),
                                status: "completed".to_string(),
                            });
                    }
                }
                Err(e) => {
                    tracing::error!("Background subagent {name} failed: {e}");
                    if let Some(store) = crate::kimi::soul::tools::SUBAGENT_STORE.get() {
                        store
                            .write()
                            .await
                            .insert(crate::kimi::subagents::store::SubagentRecord {
                                id: id2.clone(),
                                agent_type: name.clone(),
                                status: "failed".to_string(),
                            });
                    }
                }
            }
        });

        Ok(format!(
            "Background subagent {} started for task: {} (id: {})",
            subagent_type.name, task, id
        ))
    }
}
