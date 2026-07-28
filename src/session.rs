use std::sync::Arc;

use tokio::sync::Semaphore;

pub struct SessionManager {
    semaphore: Arc<Semaphore>,
}

impl SessionManager {
    pub fn new(max_sessions: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_sessions)),
        }
    }

    pub fn try_acquire(&self) -> Option<SessionGuard> {
        match self.semaphore.clone().try_acquire_owned() {
            Ok(permit) => Some(SessionGuard { _permit: permit }),
            Err(_) => None,
        }
    }
}

#[derive(Debug)]
pub struct SessionGuard {
    _permit: tokio::sync::OwnedSemaphorePermit,
}
