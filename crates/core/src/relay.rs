use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Message;

use crate::agent::Agent;

pub fn room_id(key: &[u8; 32]) -> String {
    let mut h = Sha256::new();
    h.update(b"berklin-room:");
    h.update(key);
    let d = h.finalize();
    d.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn seal(cipher: &Aes256Gcm, dev: Option<&str>, plain: &[u8]) -> Option<Vec<u8>> {
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher.encrypt(Nonce::from_slice(&nonce), plain).ok()?;
    let id = dev.unwrap_or("").as_bytes();
    let mut out = Vec::with_capacity(1 + id.len() + 12 + ct.len());
    out.push(id.len() as u8);
    out.extend_from_slice(id);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Some(out)
}

fn open(cipher: &Aes256Gcm, frame: &[u8]) -> Option<(String, String)> {
    let n = *frame.first()? as usize;
    if frame.len() < 1 + n + 12 + 16 {
        return None;
    }
    let dev = std::str::from_utf8(&frame[1..1 + n]).ok()?.to_string();
    let nonce = &frame[1 + n..1 + n + 12];
    let plain = cipher.decrypt(Nonce::from_slice(nonce), &frame[1 + n + 12..]).ok()?;
    Some((dev, String::from_utf8(plain).ok()?))
}

pub async fn run(agent: Arc<Agent>, mut out_rx: UnboundedReceiver<(Option<String>, Arc<str>)>) {
    let mut backoff = 1u64;
    loop {
        let (url, token, key) = {
            let c = agent.cfg.read();
            (c.relay_url.trim().to_string(), c.relay_token.trim().to_string(), c.key_bytes())
        };
        if url.is_empty() {
            agent.relay_online.store(false, Ordering::Relaxed);
            loop {
                tokio::select! {
                    _ = agent.relay_wake.notified() => break,
                    _ = out_rx.recv() => {}
                }
            }
            continue;
        }
        let sep = if url.contains('?') { '&' } else { '?' };
        let full = format!(
            "{url}{sep}room={}&role=agent&dev=pc&t={}",
            room_id(&key),
            urlencoding::encode(&token)
        );
        let cipher = Aes256Gcm::new_from_slice(&key).expect("key");
        let started = Instant::now();
        match tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(&full)).await {
            Ok(Ok((ws, _))) => {
                agent.relay_online.store(true, Ordering::Relaxed);
                agent.log("relay conectado");
                session(&agent, ws, &cipher, &mut out_rx).await;
                agent.relay_online.store(false, Ordering::Relaxed);
                agent.drop_remote(None);
                agent.log("relay desconectado");
            }
            Ok(Err(e)) => agent.log(&format!("relay erro: {e}")),
            Err(_) => agent.log("relay timeout"),
        }
        if started.elapsed() > Duration::from_secs(60) {
            backoff = 1;
        }
        while out_rx.try_recv().is_ok() {}
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
            _ = agent.relay_wake.notified() => { backoff = 1; continue; }
        }
        backoff = (backoff * 2).min(30);
    }
}

async fn session<S>(
    agent: &Arc<Agent>,
    ws: tokio_tungstenite::WebSocketStream<S>,
    cipher: &Aes256Gcm,
    out_rx: &mut UnboundedReceiver<(Option<String>, Arc<str>)>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut sink, mut stream) = ws.split();
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            msg = stream.next() => {
                let Some(Ok(msg)) = msg else { return };
                last_seen = Instant::now();
                match msg {
                    Message::Binary(b) => {
                        if let Some((dev, text)) = open(cipher, &b) {
                            let conn = agent.remote_conn(&dev);
                            let a = agent.clone();
                            tokio::spawn(async move { a.handle_text(conn, &text).await });
                        }
                    }
                    Message::Text(t) => {
                        if let Ok(v) = serde_json::from_str::<Value>(t.as_str()) {
                            if v.get("relay").and_then(Value::as_str) == Some("client")
                                && v.get("online").and_then(Value::as_bool) == Some(false)
                            {
                                if let Some(dev) = v.get("dev").and_then(Value::as_str) {
                                    agent.drop_remote(Some(dev));
                                }
                            }
                        }
                    }
                    Message::Close(_) => return,
                    _ => {}
                }
            }
            out = out_rx.recv() => {
                let Some((dev, text)) = out else { return };
                if let Some(frame) = seal(cipher, dev.as_deref(), text.as_bytes()) {
                    if sink.send(Message::Binary(frame.into())).await.is_err() {
                        return;
                    }
                }
            }
            _ = ping.tick() => {
                if last_seen.elapsed() > Duration::from_secs(80) {
                    return;
                }
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
            _ = agent.relay_wake.notified() => {
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
        }
    }
}
