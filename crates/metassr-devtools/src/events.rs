//! Event types streamed from the dev server to the in-page devtools panel.

use serde::Serialize;

/// Maximum size, in bytes, for captured request/response payloads.
/// Larger payloads are truncated before they are sent to the browser.
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024;

/// A debug event recorded by the dev server.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DebugEvent {
    /// A single polyglot API route call.
    ApiTrace(ApiTraceEvent),
}

impl DebugEvent {
    pub fn id(&self) -> u64 {
        match self {
            DebugEvent::ApiTrace(event) => event.id,
        }
    }

    pub fn set_id(&mut self, id: u64) {
        match self {
            DebugEvent::ApiTrace(event) => event.id = id,
        }
    }
}

/// A trace of one call to an API route handler (Node, Python or Ruby).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiTraceEvent {
    /// Monotonic id assigned by the [`DebugHub`](super::hub::DebugHub).
    pub id: u64,
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: i64,
    /// HTTP method (GET, POST, ...).
    pub method: String,
    /// Registered API route (e.g. `/api/hello`).
    pub route: String,
    /// Source file of the handler, as discovered by the scanner.
    pub file: String,
    /// Loader that executed the handler (`node`, `python`, `ruby`).
    pub loader: String,
    /// Response status, `None` when the handler failed before responding.
    pub status: Option<u16>,
    /// Wall-clock time spent inside the polyglot call, in milliseconds.
    pub duration_ms: f64,
    /// Serialized request passed to the handler (truncated).
    pub request: Option<String>,
    /// Serialized response returned by the handler (truncated).
    pub response: Option<String>,
    /// Error message when the call failed.
    pub error: Option<String>,
}

impl ApiTraceEvent {
    /// Creates a trace with no captured payloads, to be filled in as the call
    /// progresses.
    pub fn new(
        method: impl Into<String>,
        route: impl Into<String>,
        file: impl Into<String>,
        loader: impl Into<String>,
    ) -> Self {
        Self {
            id: 0,
            timestamp_ms: now_ms(),
            method: method.into(),
            route: route.into(),
            file: file.into(),
            loader: loader.into(),
            status: None,
            duration_ms: 0.0,
            request: None,
            response: None,
            error: None,
        }
    }
}

/// Current unix time in milliseconds. Falls back to 0 if the clock is before
/// the epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Truncates a serialized payload to [`MAX_PAYLOAD_BYTES`], appending a marker
/// with the number of dropped bytes. Respects UTF-8 char boundaries.
pub fn truncate_payload(payload: String) -> String {
    if payload.len() <= MAX_PAYLOAD_BYTES {
        return payload;
    }

    let mut end = MAX_PAYLOAD_BYTES;
    while end > 0 && !payload.is_char_boundary(end) {
        end -= 1;
    }

    let dropped = payload.len() - end;
    format!("{}…<truncated {dropped} bytes>", &payload[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_trace_serializes_with_kind_tag() {
        let event = ApiTraceEvent {
            id: 7,
            timestamp_ms: 1_700_000_000_000,
            method: "GET".into(),
            route: "/api/hello".into(),
            file: "src/api/hello.js".into(),
            loader: "node".into(),
            status: Some(200),
            duration_ms: 1.5,
            request: Some(r#"{"url":"/api/hello"}"#.into()),
            response: Some(r#"{"status":200}"#.into()),
            error: None,
        };

        let json = serde_json::to_value(DebugEvent::ApiTrace(event)).unwrap();

        assert_eq!(json["kind"], "api_trace");
        assert_eq!(json["id"], 7);
        assert_eq!(json["method"], "GET");
        assert_eq!(json["route"], "/api/hello");
        assert_eq!(json["loader"], "node");
        assert_eq!(json["status"], 200);
        assert!(json["error"].is_null());
    }

    #[test]
    fn new_event_has_zero_id_and_timestamp() {
        let event = ApiTraceEvent::new("POST", "/api/users", "src/api/users.py", "python");

        assert_eq!(event.id, 0);
        assert_eq!(event.status, None);
        assert!(event.timestamp_ms > 0);
    }

    #[test]
    fn truncate_payload_keeps_small_payloads_intact() {
        let payload = "small".to_string();
        assert_eq!(truncate_payload(payload.clone()), payload);
    }

    #[test]
    fn truncate_payload_marks_dropped_bytes() {
        let payload = "a".repeat(MAX_PAYLOAD_BYTES + 100);
        let truncated = truncate_payload(payload);

        assert!(truncated.len() < MAX_PAYLOAD_BYTES + 100);
        assert!(truncated.ends_with("…<truncated 100 bytes>"));
    }

    #[test]
    fn truncate_payload_respects_char_boundaries() {
        let payload = "é".repeat(MAX_PAYLOAD_BYTES);
        let truncated = truncate_payload(payload);

        assert!(truncated.starts_with('é'));
    }
}
