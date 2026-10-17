use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::Body,
    http::{header, Request, Response, StatusCode},
    middleware::Next,
};
use futures_util::{SinkExt, StreamExt};
use metassr_build::rebuilder::RebuildType;
use metassr_devtools::DebugEvent;
use serde::Serialize;
use tokio_tungstenite::tungstenite::Message;

use tokio::{
    net::TcpStream,
    sync::broadcast::{error::RecvError, Receiver},
};
use tracing::{info, warn};

/// How long the server waits for the client's `ready` frame before pushing
/// cached state anyway.
const READY_TIMEOUT: Duration = Duration::from_secs(5);

/// Messages the server pushes to browser clients over the dev WebSocket.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Page {
        path: String,
    },
    Api {
        path: String,
    },
    BuildError {
        errors: Vec<String>,
    },
    Layout,
    Component,
    Style,
    Static,
    /// A single devtools event (API trace, ...).
    Debug {
        event: DebugEvent,
    },
    /// Replay of recent devtools events for panels opened after the fact.
    DebugSnapshot {
        events: Vec<DebugEvent>,
    },
}

impl From<&RebuildType> for ServerMessage {
    fn from(rebuild_type: &RebuildType) -> Self {
        match rebuild_type {
            RebuildType::Page(path) => ServerMessage::Page {
                path: path.to_string_lossy().to_string(),
            },
            RebuildType::Api(path) => ServerMessage::Api {
                path: path.to_string_lossy().to_string(),
            },
            RebuildType::BuildError { errors } => ServerMessage::BuildError {
                errors: errors.clone(),
            },
            RebuildType::Layout => ServerMessage::Layout,
            RebuildType::Component => ServerMessage::Component,
            RebuildType::Style => ServerMessage::Style,
            RebuildType::Static => ServerMessage::Static,
        }
    }
}

pub struct LiveReloadServer {
    receiver: Receiver<RebuildType>,
    last_errors: Arc<Mutex<Option<Vec<String>>>>,
    debug_receiver: Receiver<DebugEvent>,
    debug_history: Vec<DebugEvent>,
}

impl LiveReloadServer {
    pub fn new(
        receiver: Receiver<RebuildType>,
        last_errors: Arc<Mutex<Option<Vec<String>>>>,
        debug_receiver: Receiver<DebugEvent>,
        debug_history: Vec<DebugEvent>,
    ) -> Self {
        Self {
            receiver,
            last_errors,
            debug_receiver,
            debug_history,
        }
    }

    pub async fn handle_connection(mut self, stream: TcpStream) {
        let ws_stream = tokio_tungstenite::accept_async(stream)
            .await
            .expect("Error during websocket handshake");

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // Immediately send cached state (last known errors and debug history),
        // but wait for the client's ready signal first so that the onmessage
        // handler is guaranteed to be in place before we push anything.
        let last_err_msgs = self.last_errors.lock().unwrap().clone();
        let has_pending = last_err_msgs.is_some() || !self.debug_history.is_empty();

        if has_pending {
            let _ = tokio::time::timeout(READY_TIMEOUT, async {
                while let Some(Ok(msg)) = ws_receiver.next().await {
                    if let Message::Text(text) = msg {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                            if parsed.get("type").and_then(|t| t.as_str()) == Some("ready") {
                                break;
                            }
                        }
                    }
                }
            })
            .await;

            if let Some(err_msgs) = last_err_msgs {
                let message = ServerMessage::from(&RebuildType::BuildError { errors: err_msgs });
                if let Ok(message_json) = serde_json::to_string(&message) {
                    let _ = ws_sender.send(Message::Text(message_json.into())).await;
                }
            }

            if !self.debug_history.is_empty() {
                let message = ServerMessage::DebugSnapshot {
                    events: std::mem::take(&mut self.debug_history),
                };
                if let Ok(message_json) = serde_json::to_string(&message) {
                    let _ = ws_sender.send(Message::Text(message_json.into())).await;
                }
            }
        }

        loop {
            let message = tokio::select! {
                rebuild = self.receiver.recv() => match rebuild {
                    Ok(rebuild_type) => ServerMessage::from(&rebuild_type),
                    Err(RecvError::Lagged(skipped)) => {
                        warn!("Live-reload subscriber lagged, skipped {} messages", skipped);
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                },
                debug = self.debug_receiver.recv() => match debug {
                    Ok(event) => ServerMessage::Debug { event },
                    Err(RecvError::Lagged(skipped)) => {
                        warn!("devtools subscriber lagged, skipped {} events", skipped);
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                },
            };

            let message_json = match serde_json::to_string(&message) {
                Ok(json) => json,
                Err(e) => {
                    tracing::error!("Failed to serialize LiveReload message: {}", e);
                    continue;
                }
            };

            if let Err(e) = ws_sender.send(Message::Text(message_json.into())).await {
                tracing::error!("Failed to send LiveReload message: {}", e);
                break;
            }
        }
    }
}

/// middleware to inject the live-reload.js script
pub async fn inject_live_reload_script(
    req: Request<Body>,
    next: Next,
) -> Result<Response<Body>, StatusCode> {
    let response = next.run(req).await;

    // Check if the response is HTML
    let is_html: bool = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| {
            v.to_str()
                .unwrap_or("")
                .to_lowercase()
                .contains("text/html")
        })
        .unwrap_or(false);

    if is_html {
        let (parts, body) = response.into_parts();

        let body_bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| {
            info!("Failed to read response body: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        let body_str = String::from_utf8_lossy(&body_bytes).to_string();

        // Inject scripts before </body>. The middleware is layered twice (base
        // router + post-pages router), so guard against double injection.
        const DEVTOOLS_MARKER: &str = r#"src="/__metassr/devtools.js""#;
        const INJECTED_SCRIPTS: &str = concat!(
            r#"<script src="/livereload/script.js"></script>"#,
            r#"<script src="/__metassr/devtools.js" defer></script>"#,
        );

        let modified_body = if body_str.contains(DEVTOOLS_MARKER) {
            body_str
        } else {
            body_str.replace("</body>", &format!("{INJECTED_SCRIPTS}</body>"))
        };

        return Ok(Response::builder()
            .status(parts.status)
            .header(header::CONTENT_TYPE, "text/html")
            .header(header::CACHE_CONTROL, "no-cache") // Prevent caching in dev
            .body(Body::from(modified_body))
            .unwrap());
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{header, Request, StatusCode},
        routing::get,
        Router,
    };
    use metassr_devtools::ApiTraceEvent;
    use std::path::PathBuf;
    use tower_service::Service;

    // ---- middleware tests ----

    #[tokio::test]
    async fn injects_script_into_html_responses() {
        let app = Router::new()
            .route(
                "/",
                get(|| async {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                        .body(Body::from("<html><body>Hello</body></html>"))
                        .unwrap()
                }),
            )
            .layer(axum::middleware::from_fn(inject_live_reload_script));

        let mut app = app;
        let response = Service::call(
            &mut app,
            Request::builder().uri("/").body(Body::empty()).unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-cache"
        );

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains(r#"<script src="/livereload/script.js"></script>"#));
        assert!(html.contains(r#"<script src="/__metassr/devtools.js" defer></script>"#));
    }

    #[tokio::test]
    async fn does_not_double_inject_when_layered_twice() {
        let app = Router::new()
            .route(
                "/",
                get(|| async {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/html")
                        .body(Body::from("<html><body>Hello</body></html>"))
                        .unwrap()
                }),
            )
            .layer(axum::middleware::from_fn(inject_live_reload_script))
            .layer(axum::middleware::from_fn(inject_live_reload_script));

        let mut app = app;
        let response = Service::call(
            &mut app,
            Request::builder().uri("/").body(Body::empty()).unwrap(),
        )
        .await
        .unwrap();

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8_lossy(&body);
        assert_eq!(html.matches("/livereload/script.js").count(), 1);
        assert_eq!(html.matches("/__metassr/devtools.js").count(), 1);
    }

    #[tokio::test]
    async fn leaves_non_html_responses_unchanged() {
        let app = Router::new()
            .route(
                "/json",
                get(|| async {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"ok":true}"#))
                        .unwrap()
                }),
            )
            .layer(axum::middleware::from_fn(inject_live_reload_script));

        let mut app = app;
        let response = Service::call(
            &mut app,
            Request::builder().uri("/json").body(Body::empty()).unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::CACHE_CONTROL).is_none());

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&body), r#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn does_not_inject_when_body_tag_missing() {
        let app = Router::new()
            .route(
                "/fragment",
                get(|| async {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/html")
                        .body(Body::from("<div>no body tag here</div>"))
                        .unwrap()
                }),
            )
            .layer(axum::middleware::from_fn(inject_live_reload_script));

        let mut app = app;
        let response = Service::call(
            &mut app,
            Request::builder()
                .uri("/fragment")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8_lossy(&body);
        assert!(!html.contains("livereload"));
    }

    #[tokio::test]
    async fn preserves_original_status_code() {
        let app = Router::new()
            .route(
                "/not-found",
                get(|| async {
                    Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .header(header::CONTENT_TYPE, "text/html")
                        .body(Body::from("<html><body>404</body></html>"))
                        .unwrap()
                }),
            )
            .layer(axum::middleware::from_fn(inject_live_reload_script));

        let mut app = app;
        let response = Service::call(
            &mut app,
            Request::builder()
                .uri("/not-found")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // ---- message serialization (must match what live-reload.js parses) ----

    #[test]
    fn page_message_serializes_with_type_and_path() {
        let msg = ServerMessage::from(&RebuildType::Page(PathBuf::from("src/pages/index.tsx")));
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "page");
        assert_eq!(json["path"], "src/pages/index.tsx");
    }

    #[test]
    fn api_message_serializes_with_type_and_path() {
        let msg = ServerMessage::from(&RebuildType::Api(PathBuf::from("src/api/hello.js")));
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "api");
        assert_eq!(json["path"], "src/api/hello.js");
    }

    #[test]
    fn non_page_message_serializes_without_path() {
        let msg = ServerMessage::from(&RebuildType::Layout);
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "layout");
        assert!(json["path"].is_null());
    }

    #[test]
    fn debug_message_serializes_with_event() {
        let event = DebugEvent::ApiTrace(ApiTraceEvent::new(
            "GET",
            "/api/hello",
            "src/api/hello.js",
            "node",
        ));
        let msg = ServerMessage::Debug { event };
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "debug");
        assert_eq!(json["event"]["kind"], "api_trace");
        assert_eq!(json["event"]["route"], "/api/hello");
    }

    // ---- WebSocket server ----

    fn test_server(
        receiver: Receiver<RebuildType>,
        debug_receiver: Receiver<DebugEvent>,
        debug_history: Vec<DebugEvent>,
    ) -> LiveReloadServer {
        LiveReloadServer::new(
            receiver,
            Arc::new(Mutex::new(None)),
            debug_receiver,
            debug_history,
        )
    }

    #[tokio::test]
    async fn server_sends_rebuild_message_over_websocket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (sender, _) = tokio::sync::broadcast::channel::<RebuildType>(16);
        let (debug_sender, _) = tokio::sync::broadcast::channel::<DebugEvent>(16);

        let receiver = sender.subscribe();
        let debug_receiver = debug_sender.subscribe();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let server = test_server(receiver, debug_receiver, Vec::new());
            server.handle_connection(stream).await;
        });

        let url = format!("ws://{}", addr);
        let (mut ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Page rebuild — should have type and path
        sender
            .send(RebuildType::Page(PathBuf::from("src/pages/about.tsx")))
            .unwrap();

        let msg = ws_stream.next().await.unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert_eq!(json["type"], "page");
        assert_eq!(json["path"], "src/pages/about.tsx");

        // API reload — should have type and path
        sender
            .send(RebuildType::Api(PathBuf::from("src/api/hello.js")))
            .unwrap();

        let msg = ws_stream.next().await.unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert_eq!(json["type"], "api");
        assert_eq!(json["path"], "src/api/hello.js");

        // Style rebuild — should have type only
        sender.send(RebuildType::Style).unwrap();

        let msg = ws_stream.next().await.unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert_eq!(json["type"], "style");
        assert!(json["path"].is_null());
    }

    #[tokio::test]
    async fn server_forwards_debug_events_over_websocket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (sender, _) = tokio::sync::broadcast::channel::<RebuildType>(16);
        let (debug_sender, _) = tokio::sync::broadcast::channel::<DebugEvent>(16);

        let receiver = sender.subscribe();
        let debug_receiver = debug_sender.subscribe();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let server = test_server(receiver, debug_receiver, Vec::new());
            server.handle_connection(stream).await;
        });

        let url = format!("ws://{}", addr);
        let (mut ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        debug_sender
            .send(DebugEvent::ApiTrace(ApiTraceEvent::new(
                "POST",
                "/api/users",
                "src/api/users.py",
                "python",
            )))
            .unwrap();

        let msg = ws_stream.next().await.unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert_eq!(json["type"], "debug");
        assert_eq!(json["event"]["kind"], "api_trace");
        assert_eq!(json["event"]["method"], "POST");
        assert_eq!(json["event"]["loader"], "python");
    }

    #[tokio::test]
    async fn server_replays_debug_history_after_ready() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (sender, _) = tokio::sync::broadcast::channel::<RebuildType>(16);
        let (debug_sender, _) = tokio::sync::broadcast::channel::<DebugEvent>(16);

        let history = vec![
            DebugEvent::ApiTrace(ApiTraceEvent::new(
                "GET",
                "/api/one",
                "src/api/one.js",
                "node",
            )),
            DebugEvent::ApiTrace(ApiTraceEvent::new(
                "GET",
                "/api/two",
                "src/api/two.js",
                "node",
            )),
        ];

        let receiver = sender.subscribe();
        let debug_receiver = debug_sender.subscribe();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let server = test_server(receiver, debug_receiver, history);
            server.handle_connection(stream).await;
        });

        let url = format!("ws://{}", addr);
        let (mut ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        ws_stream
            .send(Message::Text(r#"{"type":"ready"}"#.into()))
            .await
            .unwrap();

        let msg = ws_stream.next().await.unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert_eq!(json["type"], "debug_snapshot");
        assert_eq!(json["events"].as_array().unwrap().len(), 2);
        assert_eq!(json["events"][0]["route"], "/api/one");
        assert_eq!(json["events"][1]["route"], "/api/two");
    }
}
