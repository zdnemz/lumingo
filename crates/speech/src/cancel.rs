use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A flag that long-running engine calls check between units of work.
///
/// Clones share one flag. Cancelling is permanent for that flag: make a new one
/// for the next turn.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::CancelFlag;

    #[test]
    fn clones_share_one_flag() {
        let flag = CancelFlag::new();
        let other = flag.clone();
        assert!(!other.is_cancelled());
        flag.cancel();
        assert!(other.is_cancelled());
    }

    #[test]
    fn a_new_flag_starts_clear() {
        let flag = CancelFlag::new();
        flag.cancel();
        assert!(!CancelFlag::new().is_cancelled());
    }
}
