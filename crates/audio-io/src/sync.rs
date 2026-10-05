use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks a mutex and carries on if another thread panicked while holding it.
///
/// Every mutex in this crate guards plain data that stays consistent between
/// statements, so a poisoned lock is no reason to take the audio path down too.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
