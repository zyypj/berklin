use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::agent::Agent;

pub async fn serve(agent: Arc<Agent>, listener: tokio::net::TcpListener) {
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/hook/{event}", post(hook))
        .route("/tool/{name}", post(tool))
        .route("/ws", get(ws))
        .with_state(agent);
    let _ = axum::serve(listener, app).await;
}

async fn hook(State(agent): State<Arc<Agent>>, Path(event): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(v) = serde_json::from_slice::<Value>(&body) else { return StatusCode::NO_CONTENT.into_response() };
    let headless = headers.get("x-berklin-headless").map(|h| h.as_bytes() == b"1").unwrap_or(false);
    let out = agent.hook(&event, v, headless).await;
    if out.is_null() {
        StatusCode::NO_CONTENT.into_response()
    } else {
        ([("content-type", "application/json")], out.to_string()).into_response()
    }
}

async fn tool(State(agent): State<Arc<Agent>>, Path(name): Path<String>, body: Bytes) -> Response {
    let args = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let out = match agent.tool(&name, args).await {
        Ok(text) => json!({ "ok": true, "text": text }),
        Err(e) => json!({ "ok": false, "text": e }),
    };
    ([("content-type", "application/json")], out.to_string()).into_response()
}

async fn ws(State(agent): State<Arc<Agent>>, Query(q): Query<HashMap<String, String>>, up: WebSocketUpgrade) -> Response {
    let ok = q.get("t").map(|t| *t == agent.cfg.read().local_token).unwrap_or(false);
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    up.max_message_size(4 * 1024 * 1024).on_upgrade(move |socket| local_conn(agent, socket))
}

async fn local_conn(agent: Arc<Agent>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Arc<str>>();
    let id = agent.add_local_conn(tx);
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(Message::Text(m.as_ref().into())).await.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(t) => {
                let a = agent.clone();
                let text = t.to_string();
                tokio::spawn(async move { a.handle_text(id, &text).await });
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    agent.drop_conn(id);
    writer.abort();
}
