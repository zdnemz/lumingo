//! Shared, mutable capabilities. Adapters learn quirks while they work (the token
//! limit parameter, a rejected temperature) and the probe fills in the rest; both
//! write to the same handle, and `LlmClient::capabilities` returns a snapshot.

use std::sync::{Arc, Mutex, PoisonError};

use crate::types::Capabilities;

#[derive(Debug, Clone, Default)]
pub struct CapsHandle(Arc<Mutex<Capabilities>>);

impl CapsHandle {
    pub fn new(initial: Capabilities) -> Self {
        Self(Arc::new(Mutex::new(initial)))
    }

    /// Replaces everything, for example with capabilities stored from an earlier probe.
    pub fn replace(&self, capabilities: Capabilities) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = capabilities;
    }

    pub fn snapshot(&self) -> Capabilities {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Runs `change` under the lock. The lock is never held across an await point.
    pub(crate) fn update<R>(&self, change: impl FnOnce(&mut Capabilities) -> R) -> R {
        change(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}
