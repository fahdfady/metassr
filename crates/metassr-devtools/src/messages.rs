//! WebSocket messages pushed to the dev-mode browser client.

use serde::Serialize;

/// A message the live-reload server sends to the browser.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveReloadMessage {
    #[serde(rename = "type")]
    type_: String,
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<String>>,
}

impl LiveReloadMessage {
    /// A rebuild notification carrying only its type
    /// (`layout`, `component`, `style`, `static`, ...).
    pub fn simple(kind: impl Into<String>) -> Self {
        Self {
            type_: kind.into(),
            path: None,
            errors: None,
        }
    }

    /// A single page changed; `path` is root-relative.
    pub fn page(path: impl Into<String>) -> Self {
        Self {
            type_: "page".to_string(),
            path: Some(path.into()),
            errors: None,
        }
    }

    /// The build failed, carrying compiler output for the overlay.
    pub fn build_error(errors: Vec<String>) -> Self {
        Self {
            type_: "build_error".to_string(),
            path: None,
            errors: Some(errors),
        }
    }

    pub fn kind(&self) -> &str {
        &self.type_
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn to_json(message: &LiveReloadMessage) -> Value {
        serde_json::to_value(message).unwrap()
    }

    #[test]
    fn page_message_serializes_with_type_and_path() {
        let json = to_json(&LiveReloadMessage::page("src/pages/index.tsx"));

        assert_eq!(json["type"], "page");
        assert_eq!(json["path"], "src/pages/index.tsx");
        assert!(json["errors"].is_null());
    }

    #[test]
    fn build_error_message_serializes_with_errors() {
        let json = to_json(&LiveReloadMessage::build_error(vec!["boom".to_string()]));

        assert_eq!(json["type"], "build_error");
        assert!(json["path"].is_null());
        assert_eq!(json["errors"][0], "boom");
    }

    #[test]
    fn simple_message_serializes_without_path_or_errors() {
        let json = to_json(&LiveReloadMessage::simple("style"));

        assert_eq!(json["type"], "style");
        assert!(json["path"].is_null());
        assert!(json["errors"].is_null());
    }
}
