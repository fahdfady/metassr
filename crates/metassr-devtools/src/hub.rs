//! In-process hub that fans debug events out to live WebSocket clients and
//! keeps a bounded history for panels opened after the fact.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use tokio::sync::broadcast;

use crate::events::DebugEvent;

/// Number of events kept for late subscribers by default.
pub const DEFAULT_HISTORY_CAPACITY: usize = 200;

/// Broadcast buffer size. Slow clients fall behind and miss events rather
/// than blocking the server.
const CHANNEL_CAPACITY: usize = 256;

pub struct DebugHub {
    sender: broadcast::Sender<DebugEvent>,
    history: Mutex<VecDeque<DebugEvent>>,
    capacity: usize,
    next_id: AtomicU64,
}

impl DebugHub {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);

        Self {
            sender,
            history: Mutex::new(VecDeque::with_capacity(capacity.min(64))),
            capacity,
            next_id: AtomicU64::new(1),
        }
    }

    /// Subscribes to events recorded after this call. Use
    /// [`history`](Self::history) to seed a client with earlier events.
    pub fn subscribe(&self) -> broadcast::Receiver<DebugEvent> {
        self.sender.subscribe()
    }

    /// Snapshot of the most recent events, oldest first.
    pub fn history(&self) -> Vec<DebugEvent> {
        self.history.lock().unwrap().iter().cloned().collect()
    }

    /// Assigns an id, stores the event in the bounded history and broadcasts it.
    /// Recording never fails: with no subscribers, the event is only kept in
    /// the history.
    pub fn record(&self, mut event: DebugEvent) {
        event.set_id(self.next_id.fetch_add(1, Ordering::Relaxed));

        {
            let mut history = self.history.lock().unwrap();
            if self.capacity == 0 {
                return;
            }
            if history.len() == self.capacity {
                history.pop_front();
            }
            history.push_back(event.clone());
        }

        let _ = self.sender.send(event);
    }
}

impl Default for DebugHub {
    fn default() -> Self {
        Self::new(DEFAULT_HISTORY_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ApiTraceEvent;

    fn trace(route: &str) -> DebugEvent {
        DebugEvent::ApiTrace(ApiTraceEvent::new("GET", route, "src/api/x.js", "node"))
    }

    #[test]
    fn record_assigns_sequential_ids() {
        let hub = DebugHub::new(10);

        hub.record(trace("/api/a"));
        hub.record(trace("/api/b"));

        let history = hub.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id(), 1);
        assert_eq!(history[1].id(), 2);
    }

    #[test]
    fn history_is_bounded_and_keeps_latest() {
        let hub = DebugHub::new(2);

        hub.record(trace("/api/a"));
        hub.record(trace("/api/b"));
        hub.record(trace("/api/c"));

        let history = hub.history();
        assert_eq!(history.len(), 2);
        match &history[0] {
            DebugEvent::ApiTrace(event) => assert_eq!(event.route, "/api/b"),
        }
    }

    #[tokio::test]
    async fn subscribers_receive_recorded_events() {
        let hub = DebugHub::new(10);
        let mut receiver = hub.subscribe();

        hub.record(trace("/api/a"));

        let event = receiver.recv().await.unwrap();
        match event {
            DebugEvent::ApiTrace(event) => {
                assert_eq!(event.route, "/api/a");
                assert_eq!(event.id, 1);
            }
        }
    }

    #[test]
    fn recording_without_subscribers_does_not_fail() {
        let hub = DebugHub::new(10);
        hub.record(trace("/api/a"));

        assert_eq!(hub.history().len(), 1);
    }
}
