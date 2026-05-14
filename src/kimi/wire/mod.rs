pub mod server;
pub mod types;

use tokio::sync::broadcast;

use crate::kimi::wire::types::WireEvent;

pub struct WireHub {
    pub tx: broadcast::Sender<WireEvent>,
}

impl WireHub {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WireEvent> {
        self.tx.subscribe()
    }

    pub fn send(&self, event: WireEvent) -> Result<usize, broadcast::error::SendError<WireEvent>> {
        self.tx.send(event)
    }
}
