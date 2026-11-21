use std::io::{BufRead, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("BERKLIN_DATA_DIR") {
        return PathBuf::from(d);
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join(".berklin")
}

fn port() -> u16 {
    if let Some(p) = std::env::var("BERKLIN_PORT").ok().and_then(|p| p.parse().ok()) {
        return p;
    }
    std::fs::read_to_string(data_dir().join("port"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(47821)
}

fn decode_chunked(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(pos) = body.windows(2).position(|w| w == b"\r\n") else { break };
        let size = std::str::from_utf8(&body[..pos])
            .ok()
            .and_then(|s| usize::from_str_radix(s.split(';').next().unwrap_or("").trim(), 16).ok())
            .unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = pos + 2;
        if body.len() < start + size {
            out.extend_from_slice(&body[start..]);
            break;
        }
        out.extend_from_slice(&body[start..start + size]);
        body = &body[(start + size + 2).min(body.len())..];
    }
    out
}

fn post(path: &str, body: &[u8]) -> Option<(u16, Vec<u8>)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port()));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
    let _ = s.set_nodelay(true);
    let headless = if std::env::var_os("BERKLIN_HEADLESS").is_some() { "x-berklin-headless: 1\r\n" } else { "" };
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headless}\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).ok()?;
    s.write_all(body).ok()?;
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).ok()?;
    let split = resp.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&resp[..split]).ok()?;
    let status: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
    let mut payload = resp[split + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        payload = decode_chunked(&payload);
    }
    Some((status, payload))
}

fn run_hook(event: &str) {
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    if let Some((200, body)) = post(&format!("/hook/{event}"), &input) {
        let mut out = std::io::stdout();
        let _ = out.write_all(&body);
        let _ = out.flush();
    }
}

fn tools() -> Value {
    json!([
        {
            "name": "notify_user",
            "description": "Send a push notification to the user's phone and PC through the Berklin app. Use it whenever you find something the user should know now: a PR got change requests, CI broke, a long task finished with a relevant result, you are blocked. Keep it short and concrete. The user can reply from the notification; the reply arrives as a new message in this conversation. Do not notify when nothing changed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "Notification text, short and in the user's language." },
                    "title": { "type": "string", "description": "Optional short title." }
                },
                "required": ["message"]
            }
        },
        {
            "name": "ask_user",
            "description": "Ask the user a question through the Berklin app (it rings their phone) and wait for the answer. Use it when you need a decision and the user may be away from the PC. Returns the user's answer text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": { "type": "string" },
                    "options": { "type": "array", "items": { "type": "string" }, "description": "Optional suggested answers (up to 4)." }
                },
                "required": ["question"]
            }
        },
        {
            "name": "schedule_task",
            "description": "Schedule something for later in Berklin. kind \"claude\" (default) starts a new headless Claude Code session that runs `prompt` inside directory `dir` at the scheduled time (write the prompt so that run uses notify_user only when it finds something worth telling); with notify \"claude\" (default) the user is only notified when that run calls notify_user or fails, with notify \"always\" also on every completion; kind \"reminder\" only sends the user a notification with `prompt`. Give `at` (local date-time, e.g. 2026-11-21T09:00:00) for a one-shot or `cron` (5 fields, local time, e.g. \"0 9 * * 1-5\") for a recurring schedule.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "prompt": { "type": "string" },
                    "kind": { "type": "string", "enum": ["claude", "reminder"] },
                    "at": { "type": "string" },
                    "cron": { "type": "string" },
                    "dir": { "type": "string", "description": "Working directory for kind claude. Defaults to the current project." },
                    "mode": { "type": "string", "enum": ["default", "acceptEdits", "plan", "auto", "bypassPermissions"] },
                    "notify": { "type": "string", "enum": ["claude", "always"] },
                    "chrome": { "type": "boolean", "description": "Give the scheduled run Claude in Chrome (the user's Chrome with the extension must be open). Use for web apps without an API, like Monday through the browser." }
                },
                "required": ["prompt"]
            }
        },
        {
            "name": "list_schedules",
            "description": "List the schedules stored in Berklin.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

fn call_tool(name: &str, mut args: Value) -> Value {
    if let Value::Object(m) = &mut args {
        if let Ok(cwd) = std::env::current_dir() {
            m.entry("cwd").or_insert(Value::String(cwd.to_string_lossy().into_owned()));
        }
    } else {
        args = json!({});
    }
    let body = serde_json::to_vec(&args).unwrap_or_default();
    let (text, ok) = match post(&format!("/tool/{name}"), &body) {
        Some((200, b)) => {
            let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
            (
                v.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                v.get("ok").and_then(Value::as_bool).unwrap_or(false),
            )
        }
        _ => ("O app Berklin não está aberto no PC.".to_string(), false),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": !ok })
}

fn run_mcp() {
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let send = {
        let out = out.clone();
        move |v: Value| {
            let mut o = out.lock().unwrap();
            let _ = writeln!(o, "{v}");
            let _ = o.flush();
        }
    };
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let Some(id) = id else { continue };
        match method.as_str() {
            "initialize" => {
                let version = msg.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18").to_string();
                send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": version,
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "berklin", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": "Berklin connects you to the user's phone. Use notify_user to tell them about anything important you find (they can reply from the notification), ask_user when you need their decision, and schedule_task when they ask you to do or remind something later."
                    }
                }));
            }
            "tools/list" => send(json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools() } })),
            "tools/call" => {
                let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("").to_string();
                let args = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                let send = send.clone();
                std::thread::spawn(move || {
                    let result = call_tool(&name, args);
                    send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
                });
            }
            "ping" => send(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
            _ => send(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "method not found" } })),
        }
    }
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg == "mcp" {
        run_mcp();
    } else if !arg.is_empty() {
        run_hook(&arg);
    }
}
