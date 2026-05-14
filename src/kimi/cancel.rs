use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct CancelToken {
    cancelled: AtomicBool,
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}
