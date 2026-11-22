//! Client for the legacy MCP HTTP+SSE transport (protocol 2024-11-05), which
//! rmcp 1.x no longer ships. The client opens a long-lived `GET <url>` event
//! stream; the first event (`endpoint`) names the URL to `POST` requests to,
//! and every server message arrives as a `message` event on the stream.
//!
//! Exposed as a `(Sink, Stream)` pair so it plugs into rmcp's
//! `IntoTransport` via `SinkStreamTransport`.

use std::collections::HashMap;
use std::time::Duration;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use http::{HeaderMap, HeaderName, HeaderValue};
use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use tokio::sync::oneshot;

use crate::core::error::AppError;

const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(15);

pub type SseSink = UnboundedSender<TxJsonRpcMessage<RoleClient>>;
pub type SseStream = UnboundedReceiver<RxJsonRpcMessage<RoleClient>>;

pub async fn connect(
    url: &str,
    headers: HashMap<HeaderName, HeaderValue>,
) -> Result<(SseSink, SseStream), AppError> {
    let base = reqwest::Url::parse(url)
        .map_err(|e| AppError::Validation(format!("invalid MCP SSE url '{url}': {e}")))?;

    let mut header_map = HeaderMap::new();
    for (k, v) in headers {
        header_map.insert(k, v);
    }
    let client = reqwest::Client::builder()
        .default_headers(header_map)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Tool(format!("building MCP SSE client: {e}")))?;

    let resp = client
        .get(base.clone())
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await
        .map_err(|e| AppError::Tool(format!("connecting to MCP SSE endpoint {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Tool(format!(
            "MCP SSE endpoint {url} returned {}",
            resp.status()
        )));
    }

    let (in_tx, in_rx) = unbounded::<RxJsonRpcMessage<RoleClient>>();
    let (endpoint_tx, endpoint_rx) = oneshot::channel::<String>();

    let reader = tokio::spawn(read_events(resp, in_tx, endpoint_tx));

    let endpoint = match tokio::time::timeout(ENDPOINT_TIMEOUT, endpoint_rx).await {
        Ok(Ok(e)) => e,
        _ => {
            reader.abort();
            return Err(AppError::Tool(format!(
                "MCP SSE endpoint {url} did not send an `endpoint` event"
            )));
        }
    };
    let post_url = base.join(&endpoint).map_err(|e| {
        AppError::Tool(format!(
            "invalid MCP SSE message endpoint '{endpoint}': {e}"
        ))
    })?;
    if post_url.origin() != base.origin() {
        reader.abort();
        return Err(AppError::Tool(format!(
            "MCP SSE server advertised a cross-origin message endpoint: {post_url}"
        )));
    }

    let (out_tx, mut out_rx) = unbounded::<TxJsonRpcMessage<RoleClient>>();
    tokio::spawn(async move {
        use futures::StreamExt;
        while let Some(msg) = out_rx.next().await {
            match client.post(post_url.clone()).json(&msg).send().await {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => tracing::warn!(status = %r.status(), "MCP SSE POST rejected"),
                Err(e) => tracing::warn!(error = %e, "MCP SSE POST failed"),
            }
        }
        reader.abort();
    });

    Ok((out_tx, in_rx))
}

async fn read_events(
    mut resp: reqwest::Response,
    in_tx: UnboundedSender<RxJsonRpcMessage<RoleClient>>,
    endpoint_tx: oneshot::Sender<String>,
) {
    let mut endpoint_tx = Some(endpoint_tx);
    let mut buf: Vec<u8> = Vec::new();
    let mut event = String::new();
    let mut data = String::new();

    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(error = %e, "MCP SSE stream error");
                return;
            }
        };
        buf.extend_from_slice(&chunk);

        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\n', '\r']);

            if line.is_empty() {
                if !data.is_empty() {
                    let name = if event.is_empty() {
                        "message"
                    } else {
                        event.as_str()
                    };
                    match name {
                        "endpoint" => {
                            if let Some(tx) = endpoint_tx.take() {
                                let _ = tx.send(data.clone());
                            }
                        }
                        "message" => match serde_json::from_str(&data) {
                            Ok(msg) => {
                                if in_tx.unbounded_send(msg).is_err() {
                                    return;
                                }
                            }
                            Err(e) => tracing::warn!(error = %e, "undecodable MCP SSE message"),
                        },
                        _ => {}
                    }
                }
                event.clear();
                data.clear();
            } else if let Some(v) = line.strip_prefix("event:") {
                event = v.trim_start().to_string();
            } else if let Some(v) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(v.strip_prefix(' ').unwrap_or(v));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap as AxumHeaders, StatusCode};
    use axum::response::IntoResponse;
    use axum::response::sse::{Event, Sse};
    use axum::routing::{get, post};
    use futures::StreamExt;
    use tokio::net::TcpListener;

    /// What the fake legacy-SSE MCP server saw, and what it will say next.
    #[derive(Clone)]
    struct Fake {
        endpoint: String,
        events: Arc<Mutex<Option<futures::channel::mpsc::UnboundedReceiver<String>>>>,
        posts: Arc<Mutex<Vec<serde_json::Value>>>,
        get_headers: Arc<Mutex<Vec<(String, String)>>>,
    }

    async fn sse(State(fake): State<Fake>, headers: AxumHeaders) -> impl IntoResponse {
        *fake.get_headers.lock().unwrap() = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect();
        let rx = fake.events.lock().unwrap().take().expect("one stream");
        let first = futures::stream::once(futures::future::ready(Ok::<_, Infallible>(
            Event::default()
                .event("endpoint")
                .data(fake.endpoint.clone()),
        )));
        let rest = rx.map(|data| Ok::<_, Infallible>(Event::default().event("message").data(data)));
        Sse::new(first.chain(rest))
    }

    async fn messages(State(fake): State<Fake>, body: axum::body::Bytes) -> StatusCode {
        fake.posts
            .lock()
            .unwrap()
            .push(serde_json::from_slice(&body).expect("json body"));
        StatusCode::ACCEPTED
    }

    async fn serve(
        endpoint: &str,
    ) -> (
        String,
        Fake,
        futures::channel::mpsc::UnboundedSender<String>,
    ) {
        let (tx, rx) = unbounded::<String>();
        let fake = Fake {
            endpoint: endpoint.to_string(),
            events: Arc::new(Mutex::new(Some(rx))),
            posts: Arc::default(),
            get_headers: Arc::default(),
        };
        let app = Router::new()
            .route("/sse", get(sse))
            .route("/messages", post(messages))
            .with_state(fake.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, fake, tx)
    }

    #[tokio::test]
    async fn relays_messages_both_ways_after_the_endpoint_handshake() {
        use futures::SinkExt;

        let (base, fake, server_tx) = serve("/messages?session=abc").await;
        let (mut sink, mut stream) = connect(&format!("{base}/sse"), HashMap::new())
            .await
            .expect("connects");

        // Client -> server goes out as a POST to the advertised endpoint.
        let ping: TxJsonRpcMessage<RoleClient> = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }))
        .unwrap();
        sink.send(ping).await.unwrap();
        for _ in 0..50 {
            if !fake.posts.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let posts = fake.posts.lock().unwrap().clone();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0]["method"], "notifications/initialized");

        // Server -> client arrives as a `message` event on the stream.
        server_tx
            .unbounded_send(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/tools/list_changed",
                })
                .to_string(),
            )
            .unwrap();
        let received = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("a message within 5s")
            .expect("stream open");
        let value = serde_json::to_value(&received).unwrap();
        assert_eq!(value["method"], "notifications/tools/list_changed");
    }

    #[tokio::test]
    async fn sends_the_configured_headers_on_the_event_stream_request() {
        let (base, fake, _server_tx) = serve("/messages").await;
        let mut headers = HashMap::new();
        headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer sekret"),
        );
        connect(&format!("{base}/sse"), headers)
            .await
            .expect("connects");

        let seen = fake.get_headers.lock().unwrap().clone();
        assert!(
            seen.iter()
                .any(|(k, v)| k == "authorization" && v == "Bearer sekret"),
            "headers seen by the server: {seen:?}"
        );
        assert!(
            seen.iter()
                .any(|(k, v)| k == "accept" && v.contains("text/event-stream")),
            "the stream must be requested as text/event-stream: {seen:?}"
        );
    }

    /// A server that names another origin as the message endpoint could make
    /// this client send its auth headers and every JSON-RPC message elsewhere.
    #[tokio::test]
    async fn refuses_a_cross_origin_message_endpoint() {
        let (base, _fake, _server_tx) = serve("http://evil.example/messages").await;
        let err = connect(&format!("{base}/sse"), HashMap::new())
            .await
            .expect_err("cross-origin endpoint must be refused");
        assert!(err.to_string().contains("cross-origin"), "{err}");
    }

    #[tokio::test]
    async fn a_non_success_response_is_an_error_naming_the_status() {
        async fn unauthorized() -> StatusCode {
            StatusCode::UNAUTHORIZED
        }
        let app = Router::new().route("/sse", get(unauthorized));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let err = connect(&format!("{base}/sse"), HashMap::new())
            .await
            .expect_err("401 must fail");
        assert!(err.to_string().contains("401"), "{err}");
    }

    #[tokio::test]
    async fn an_invalid_url_is_a_validation_error() {
        let err = connect("not a url", HashMap::new()).await.unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "{err:?}");
    }
}
