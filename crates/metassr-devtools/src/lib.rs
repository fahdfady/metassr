//! Debug event hub and in-page devtools protocol for MetaSSR development mode.
//!
//! The dev server creates a single [`DebugHub`] and shares it with the code
//! paths that know interesting things (today: the polyglot API handler). Events
//! recorded on the hub are broadcast to the in-page devtools panel over the
//! live-reload WebSocket, with a bounded history for panels opened later.

pub mod events;
pub mod hub;

pub use events::{truncate_payload, ApiTraceEvent, DebugEvent, MAX_PAYLOAD_BYTES};
pub use hub::{DebugHub, DEFAULT_HISTORY_CAPACITY};
