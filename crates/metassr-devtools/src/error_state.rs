//! Shared cache of the last build errors.
//!
//! The rebuilder owns the underlying `Arc<Mutex<..>>` slot; this crate wraps it
//! so the live-reload server can prime the browser overlay for clients that
//! connect after a failed build.

use std::sync::{Arc, Mutex};

/// Cheap-to-clone handle around the rebuilder's last-error slot.
#[derive(Clone, Default)]
pub struct ErrorState(Arc<Mutex<Option<Vec<String>>>>);

impl ErrorState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The most recent build errors, if any.
    pub fn get(&self) -> Option<Vec<String>> {
        self.0.lock().unwrap().clone()
    }
}

impl From<Arc<Mutex<Option<Vec<String>>>>> for ErrorState {
    fn from(inner: Arc<Mutex<Option<Vec<String>>>>) -> Self {
        Self(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_through_to_the_shared_slot() {
        let slot = Arc::new(Mutex::new(None));
        let state = ErrorState::from(Arc::clone(&slot));

        assert_eq!(state.get(), None);

        *slot.lock().unwrap() = Some(vec!["boom".to_string()]);
        assert_eq!(state.get(), Some(vec!["boom".to_string()]));
    }

    #[test]
    fn clones_share_the_same_slot() {
        let state = ErrorState::from(Arc::new(Mutex::new(Some(vec!["x".to_string()]))));
        let clone = state.clone();

        assert_eq!(clone.get(), state.get());
    }

    #[test]
    fn new_state_is_empty() {
        assert_eq!(ErrorState::new().get(), None);
    }
}
