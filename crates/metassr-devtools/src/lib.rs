//! Dev-mode browser tooling for MetaSSR.
//!
//! This crate owns the client script that powers development mode (live reload
//! and the compile-error overlay), the WebSocket protocol that feeds it, and
//! the error state shared with the build rebuilder.

pub mod error_state;
pub mod messages;
pub mod server;

pub use error_state::ErrorState;
pub use messages::LiveReloadMessage;
pub use server::{inject_live_reload_script, LiveReloadServer};

/// Route the live-reload client is served from in development mode.
pub const LIVE_RELOAD_SCRIPT_PATH: &str = "/livereload/script.js";

/// The raw live-reload client script, containing the `__WS_PORT__` placeholder.
pub const LIVE_RELOAD_SCRIPT: &str = include_str!("scripts/live-reload.js");

/// The live-reload client with its WebSocket port substituted.
pub fn live_reload_script(ws_port: u16) -> String {
    LIVE_RELOAD_SCRIPT.replace("__WS_PORT__", &ws_port.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_ws_port_placeholder() {
        let script = live_reload_script(4321);

        assert!(script.contains("ws://localhost:4321"));
        assert!(!script.contains("__WS_PORT__"));
    }
}
