use std::collections::VecDeque;

use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: String,
    pub level: NotificationLevel,
    pub message: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
    Success,
}

pub struct NotificationManager {
    notifications: RwLock<VecDeque<Notification>>,
    max_size: usize,
}

impl NotificationManager {
    pub fn new(max_size: usize) -> Self {
        Self {
            notifications: RwLock::new(VecDeque::new()),
            max_size,
        }
    }

    pub async fn push(&self, notification: Notification) {
        let mut notifications = self.notifications.write().await;
        notifications.push_back(notification);
        while notifications.len() > self.max_size {
            notifications.pop_front();
        }
    }

    pub async fn list(&self) -> Vec<Notification> {
        self.notifications.read().await.iter().cloned().collect()
    }

    pub async fn clear(&self) {
        self.notifications.write().await.clear();
    }
}
