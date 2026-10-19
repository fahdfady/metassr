//! WebSocket server that pushes rebuild notifications and cached build errors
//! to the browser, plus the middleware that injects the client script into HTML
//! responses.

use axum::{
    body::Body,
    http::{header, Request, Response, StatusCode},
    middleware::Next,
};
use futures_util::{SinkExt, StreamExt};
use tokio::{net::TcpStream, sync::broadcast::Receiver};
use tokio_tungstenite::tungstenite::Message;
use tracing::info;

use crate::{error_state::ErrorState, messages::LiveReloadMessage, LIVE_RELOAD_SCRIPT_PATH};

/// Converts a rebuild event into the message sent to the browser. Kept as a
/// function pointer so this crate does not need to depend on the build crate.
pub type MessageMapper<E> = fn(&E) -> LiveReloadMessage;

pub struct LiveReloadServer<E> {
    receiver: Receiver<E>,
    errors: ErrorState,
    to_message: MessageMapper<E>,
}

impl<E: Clone> LiveReloadServer<E> {
    pub fn new(receiver: Receiver<E>, errors: ErrorState, to_message: MessageMapper<E>) -> Self {
        Self {
            receiver,
            errors,
            to_message,
        }
    }

    pub async fn handle_connection(mut self, stream: TcpStream) {
        let ws_stream = tokio_tungstenite::accept_async(stream)
            .await
            .expect("Error during websocket handshake");

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // immediately send the last known error if there is one on handshake,
        // but wait for the client's ready signal first so that the
        // onmessage handler is guaranteed to be in place before we push anything
        let last_err_msgs = self.errors.get();
        if let Some(err_msgs) = last_err_msgs {
            // drain incoming frames until the client sends {type:"ready"}
            'ready: while let Some(msg_result) = ws_receiver.next().await {
                if let Ok(Message::Text(text)) = msg_result {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                        if parsed.get("type").and_then(|t| t.as_str()) == Some("ready") {
                            break 'ready;
                        }
                    }
                }
            }
            let message = LiveReloadMessage::build_error(err_msgs);
            if let Ok(message_json) = serde_json::to_string(&message) {
                let _ = ws_sender.send(Message::Text(message_json.into())).await;
            }
        }

        while let Ok(rebuild_type) = self.receiver.recv().await {
            let message = (self.to_message)(&rebuild_type);
            let message_json = serde_json::to_string(&message).unwrap();

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

        // Inject script before </body> or append if </body> is missing
        let script_tag = format!(r#"<script src="{LIVE_RELOAD_SCRIPT_PATH}"></script>"#);
        let modified_body = body_str.replace("</body>", &format!("{script_tag}</body>"));

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
    use std::sync::{Arc, Mutex};
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

    // ---- WebSocket server ----

    fn echo(event: &String) -> LiveReloadMessage {
        LiveReloadMessage::simple(event.clone())
    }

    #[tokio::test]
    async fn server_forwards_events_over_websocket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (sender, _) = tokio::sync::broadcast::channel::<String>(16);
        let receiver = sender.subscribe();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let server =
                LiveReloadServer::new(receiver, ErrorState::new(), echo as MessageMapper<String>);
            server.handle_connection(stream).await;
        });

        let url = format!("ws://{}", addr);
        let (mut ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        sender.send("style".to_string()).unwrap();

        let msg = ws_stream.next().await.unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert_eq!(json["type"], "style");
    }

    #[tokio::test]
    async fn server_sends_cached_error_after_ready() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (sender, _) = tokio::sync::broadcast::channel::<String>(16);
        let receiver = sender.subscribe();
        let errors = ErrorState::from(Arc::new(Mutex::new(Some(vec!["boom".to_string()]))));
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let server = LiveReloadServer::new(receiver, errors, echo as MessageMapper<String>);
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
        assert_eq!(json["type"], "build_error");
        assert_eq!(json["errors"][0], "boom");
    }
}
