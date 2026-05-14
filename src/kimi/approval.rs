use std::collections::HashMap;

use tokio::sync::{RwLock, mpsc, oneshot};
use uuid::Uuid;

use crate::kimi::error::Result;

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub id: String,
    pub tool_name: String,
    pub arguments: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ApprovalResponse {
    pub request_id: String,
    pub approved: bool,
    pub reason: Option<String>,
}

pub struct ApprovalRuntime {
    yolo: RwLock<bool>,
    per_action: RwLock<HashMap<String, bool>>,
    pending_tx: mpsc::Sender<ApprovalRequest>,
    pending_rx: RwLock<mpsc::Receiver<ApprovalRequest>>,
    responses: RwLock<HashMap<String, oneshot::Sender<ApprovalResponse>>>,
}

impl ApprovalRuntime {
    pub fn new() -> Self {
        let (pending_tx, pending_rx) = mpsc::channel(100);
        Self {
            yolo: RwLock::new(false),
            per_action: RwLock::new(HashMap::new()),
            pending_tx,
            pending_rx: RwLock::new(pending_rx),
            responses: RwLock::new(HashMap::new()),
        }
    }

    pub async fn set_yolo(&self, yolo: bool) {
        *self.yolo.write().await = yolo;
    }

    pub async fn set_per_action(&self, action: String, approved: bool) {
        self.per_action.write().await.insert(action, approved);
    }

    pub async fn is_yolo(&self) -> bool {
        *self.yolo.read().await
    }

    pub async fn get_per_action(&self) -> HashMap<String, bool> {
        self.per_action.read().await.clone()
    }

    pub async fn request_approval(
        &self,
        tool_name: String,
        arguments: HashMap<String, serde_json::Value>,
    ) -> Result<ApprovalResponse> {
        let yolo = *self.yolo.read().await;
        let per_action = self.per_action.read().await.get(&tool_name).copied();

        if yolo || per_action == Some(true) {
            let id = Uuid::new_v4().to_string();
            return Ok(ApprovalResponse {
                request_id: id,
                approved: true,
                reason: None,
            });
        }

        let id = Uuid::new_v4().to_string();
        let req = ApprovalRequest {
            id: id.clone(),
            tool_name,
            arguments,
        };

        let (tx, rx) = oneshot::channel();
        self.responses.write().await.insert(id.clone(), tx);
        self.pending_tx.send(req).await.ok();

        let resp = rx
            .await
            .map_err(|_| crate::kimi::error::MekaiError::ApprovalDenied)?;
        Ok(resp)
    }

    pub async fn approve(
        &self,
        request_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<()> {
        let tx = self.responses.write().await.remove(request_id);
        if let Some(tx) = tx {
            let _ = tx.send(ApprovalResponse {
                request_id: request_id.to_string(),
                approved,
                reason,
            });
        }
        Ok(())
    }

    pub async fn next_pending(&self) -> Option<ApprovalRequest> {
        self.pending_rx.write().await.recv().await
    }

    pub async fn has_pending(&self) -> bool {
        !self.responses.read().await.is_empty()
    }
}

impl Default for ApprovalRuntime {
    fn default() -> Self {
        Self::new()
    }
}
