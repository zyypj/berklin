use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime};

use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::Value;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::model::{clip, one_line, Item};

const RESULT_MAX: usize = 16_000;
const INPUT_MAX: usize = 24_000;
const TEXT_MAX: usize = 60_000;
const INDEX_DAYS: u64 = 30;

#[derive(Default, Clone, Serialize, Deserialize, Debug)]
pub struct Meta {
    pub custom_title: Option<String>,
    pub summary: Option<String>,
    pub agent_name: Option<String>,
    pub first_prompt: Option<String>,
    pub cwd: String,
    pub model: Option<String>,
    pub tokens: Option<u64>,
    pub preview: String,
    pub last_ts: i64,
    pub last_kind: String,
    pub next_i: u32,
}

impl Meta {
    pub fn title(&self) -> String {
        for t in [&self.custom_title, &self.summary, &self.agent_name, &self.first_prompt] {
            if let Some(t) = t {
                if !t.trim().is_empty() {
                    return one_line(t, 90);
                }
            }
        }
        "Sessão sem título".into()
    }
}

#[derive(Deserialize)]
struct Line<'a> {
    #[serde(rename = "type", borrow, default)]
    kind: Option<Cow<'a, str>>,
    #[serde(rename = "isSidechain", default)]
    sidechain: bool,
    #[serde(rename = "isMeta", default)]
    is_meta: bool,
    #[serde(rename = "isCompactSummary", default)]
    compact: bool,
    #[serde(borrow, default)]
    cwd: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    timestamp: Option<Cow<'a, str>>,
    #[serde(rename = "customTitle", default)]
    custom_title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(rename = "agentName", default)]
    agent_name: Option<String>,
    #[serde(borrow, default)]
    message: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct Msg {
    #[serde(default)]
    content: Value,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

const SKIP_PREFIXES: [&[u8]; 6] = [
    b"{\"type\":\"api-request",
    b"{\"type\":\"file-history",
    b"{\"type\":\"queue-operation",
    b"{\"type\":\"tools-used",
    b"{\"type\":\"atis",
    b"{\"type\":\"bridge-session",
];

fn parse_ts(ts: Option<&str>) -> Option<i64> {
    ts.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|d| d.timestamp_millis())
}

fn clip_value(v: &mut Value) {
    match v {
        Value::String(s) => {
            if s.len() > INPUT_MAX {
                *s = clip(s, INPUT_MAX);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(clip_value),
        Value::Object(o) => o.values_mut().for_each(clip_value),
        _ => {}
    }
}

fn flatten_result(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let mut out = String::new();
            for p in parts {
                match p.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(p.get("text").and_then(Value::as_str).unwrap_or(""));
                    }
                    Some("image") => out.push_str("[imagem]"),
                    _ => {}
                }
            }
            out
        }
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn between<'s>(s: &'s str, open: &str, close: &str) -> Option<&'s str> {
    let a = s.find(open)? + open.len();
    let b = s[a..].find(close)? + a;
    Some(&s[a..b])
}

const INJECT_MARK: &str = "[Berklin] Mensagem do usu";

fn injected_text(s: &str) -> Option<&str> {
    let at = s.find(INJECT_MARK)?;
    let rest = &s[at..];
    let body = rest.split_once(':').map(|(_, b)| b).unwrap_or(rest);
    Some(body.trim())
}

fn find_injected(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => injected_text(s).map(str::to_string),
        Value::Array(a) => a.iter().find_map(find_injected),
        Value::Object(o) => o.values().find_map(find_injected),
        _ => None,
    }
}

fn user_text_item(text: &str, ts: i64, out: &mut Vec<Item>) {
    if let Some(msg) = injected_text(text) {
        let mut it = Item::new("user", ts);
        it.text = Some(clip(msg, TEXT_MAX));
        out.push(it);
        return;
    }
    let t = text.trim();
    if t.is_empty() || t.starts_with("<system-reminder>") {
        return;
    }
    if t.starts_with("[Request interrupted") {
        let mut it = Item::new("info", ts);
        it.text = Some("Interrompido pelo usuário".into());
        out.push(it);
        return;
    }
    if t.starts_with('<') {
        if let Some(name) = between(t, "<command-name>", "</command-name>") {
            let args = between(t, "<command-args>", "</command-args>").unwrap_or("").trim();
            let mut it = Item::new("info", ts);
            it.text = Some(if args.is_empty() { name.to_string() } else { format!("{name} {args}") });
            out.push(it);
            return;
        }
        if let Some(body) = between(t, "<local-command-stdout>", "</local-command-stdout>") {
            if !body.trim().is_empty() {
                let mut it = Item::new("info", ts);
                it.text = Some(clip(body.trim(), 2000));
                out.push(it);
            }
            return;
        }
        if t.starts_with("<local-command") || t.starts_with("<command-") || t.starts_with("<bash-") || t.starts_with("<task-notification") {
            return;
        }
    }
    let mut it = Item::new("user", ts);
    it.text = Some(clip(t, TEXT_MAX));
    out.push(it);
}

fn memchr_ascii(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

pub fn parse_line(raw: &[u8], meta: &mut Meta, out: &mut Vec<Item>) {
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    if raw.len() < 2 || SKIP_PREFIXES.iter().any(|p| raw.starts_with(p)) {
        return;
    }
    let Ok(line) = serde_json::from_slice::<Line>(raw) else { return };
    if let Some(c) = &line.cwd {
        if meta.cwd != c.as_ref() {
            meta.cwd = c.to_string();
        }
    }
    let ts = parse_ts(line.timestamp.as_deref()).unwrap_or(meta.last_ts);
    let kind = line.kind.as_deref().unwrap_or("");
    let start = out.len();
    match kind {
        "custom-title" => meta.custom_title = line.custom_title,
        "summary" => meta.summary = line.summary,
        "agent-name" => meta.agent_name = line.agent_name,
        "attachment" | "system" if !line.sidechain && memchr_ascii(raw, INJECT_MARK.as_bytes()) => {
            let Ok(v) = serde_json::from_slice::<Value>(raw) else { return };
            let Some(msg) = find_injected(&v) else { return };
            let mut it = Item::new("user", ts);
            it.text = Some(clip(&msg, TEXT_MAX));
            out.push(it);
        }
        "user" | "assistant" if !line.sidechain => {
            let Some(raw_msg) = line.message else { return };
            let Ok(msg) = serde_json::from_str::<Msg>(raw_msg.get()) else { return };
            if kind == "user" {
                if line.is_meta {
                    return;
                }
                if line.compact {
                    let mut it = Item::new("info", ts);
                    it.text = Some("Conversa compactada".into());
                    out.push(it);
                } else {
                    match &msg.content {
                        Value::String(s) => user_text_item(s, ts, out),
                        Value::Array(blocks) => {
                            for b in blocks {
                                match b.get("type").and_then(Value::as_str) {
                                    Some("text") => user_text_item(b.get("text").and_then(Value::as_str).unwrap_or(""), ts, out),
                                    Some("tool_result") => {
                                        let mut it = Item::new("result", ts);
                                        it.tid = b.get("tool_use_id").and_then(Value::as_str).map(str::to_string);
                                        it.text = Some(clip(&flatten_result(b.get("content").unwrap_or(&Value::Null)), RESULT_MAX));
                                        if b.get("is_error").and_then(Value::as_bool) == Some(true) {
                                            it.err = Some(true);
                                        }
                                        out.push(it);
                                    }
                                    Some("image") => {
                                        let mut it = Item::new("info", ts);
                                        it.text = Some("[imagem enviada]".into());
                                        out.push(it);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                if let Some(m) = &msg.model {
                    if !m.starts_with('<') {
                        meta.model = Some(m.clone());
                    }
                }
                if let Some(u) = &msg.usage {
                    let total = u.input_tokens + u.output_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens;
                    if total > 0 {
                        meta.tokens = Some(total);
                    }
                }
                if let Value::Array(blocks) = &msg.content {
                    for b in blocks {
                        match b.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                let t = b.get("text").and_then(Value::as_str).unwrap_or("").trim();
                                if !t.is_empty() {
                                    let mut it = Item::new("text", ts);
                                    it.text = Some(clip(t, TEXT_MAX));
                                    out.push(it);
                                }
                            }
                            Some("thinking") => {
                                let t = b.get("thinking").and_then(Value::as_str).unwrap_or("").trim();
                                if !t.is_empty() {
                                    let mut it = Item::new("think", ts);
                                    it.text = Some(clip(t, TEXT_MAX));
                                    out.push(it);
                                }
                            }
                            Some("tool_use") => {
                                let mut it = Item::new("tool", ts);
                                it.name = b.get("name").and_then(Value::as_str).map(str::to_string);
                                it.tid = b.get("id").and_then(Value::as_str).map(str::to_string);
                                let mut input = b.get("input").cloned().unwrap_or(Value::Null);
                                clip_value(&mut input);
                                it.input = Some(input);
                                out.push(it);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        _ => return,
    }
    if ts > meta.last_ts {
        meta.last_ts = ts;
    }
    for it in out[start..].iter_mut() {
        it.i = meta.next_i;
        meta.next_i += 1;
        match it.k {
            "user" => {
                if meta.first_prompt.is_none() {
                    meta.first_prompt = it.text.as_deref().map(|t| one_line(t, 200));
                }
            }
            "text" => meta.preview = it.text.as_deref().map(|t| one_line(t, 160)).unwrap_or_default(),
            _ => {}
        }
        meta.last_kind = it.k.to_string();
    }
}

pub enum TxCmd {
    Fs(notify::Result<notify::Event>),
    Load { sid: String, reply: oneshot::Sender<Vec<Item>> },
    Track(PathBuf),
}

pub enum TxOut {
    Bulk(Vec<(String, Meta, i64)>),
    Meta(String, Meta, i64),
    Items(String, Vec<Item>),
    Loaded(String, Vec<Item>, oneshot::Sender<Vec<Item>>),
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    path: PathBuf,
    size: u64,
    offset: u64,
    meta: Meta,
}

struct FileState {
    path: PathBuf,
    offset: u64,
    meta: Meta,
    mtime: i64,
}

pub fn projects_root() -> PathBuf {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"));
    base.join("projects")
}

fn mtime_ms(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn session_file(root: &Path, path: &Path) -> Option<String> {
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return None;
    }
    if path.parent()?.parent()? != root {
        return None;
    }
    Some(path.file_stem()?.to_str()?.to_string())
}

fn read_lines_from(path: &Path, offset: u64, until: Option<u64>, meta: &mut Meta, out: &mut Vec<Item>) -> std::io::Result<u64> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let reader: Box<dyn Read> = match until {
        Some(end) => Box::new(f.take(end.saturating_sub(offset))),
        None => Box::new(f),
    };
    let mut r = BufReader::with_capacity(256 * 1024, reader);
    let mut buf = Vec::with_capacity(64 * 1024);
    let mut consumed = offset;
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            break;
        }
        consumed += n as u64;
        parse_line(&buf[..buf.len() - 1], meta, out);
    }
    Ok(consumed)
}

pub struct Transcripts {
    root: PathBuf,
    cache_path: PathBuf,
    files: HashMap<String, FileState>,
    out: UnboundedSender<TxOut>,
    dirty: bool,
}

impl Transcripts {
    pub fn spawn(cache_path: PathBuf, out: UnboundedSender<TxOut>, tx: Sender<TxCmd>, rx: Receiver<TxCmd>) {
        std::thread::Builder::new()
            .name("berklin-transcripts".into())
            .spawn(move || {
                let mut t = Transcripts { root: projects_root(), cache_path, files: HashMap::new(), out, dirty: false };
                t.initial_scan();
                let _ = std::fs::create_dir_all(&t.root);
                let watch_tx = tx.clone();
                let watcher = notify::recommended_watcher(move |res| {
                    let _ = watch_tx.send(TxCmd::Fs(res));
                });
                let _watcher = match watcher {
                    Ok(mut w) => {
                        let _ = w.watch(&t.root, RecursiveMode::Recursive);
                        Some(w)
                    }
                    Err(_) => None,
                };
                t.run(rx);
            })
            .expect("spawn transcripts thread");
    }

    fn initial_scan(&mut self) {
        let cache: Vec<CacheEntry> = std::fs::read(&self.cache_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut cached: HashMap<PathBuf, CacheEntry> = cache.into_iter().map(|e| (e.path.clone(), e)).collect();
        let cutoff = SystemTime::now() - Duration::from_secs(INDEX_DAYS * 86400);
        let mut bulk = Vec::new();
        let Ok(dirs) = std::fs::read_dir(&self.root) else { return };
        for dir in dirs.flatten() {
            let Ok(entries) = std::fs::read_dir(dir.path()) else { continue };
            for e in entries.flatten() {
                let path = e.path();
                let Some(sid) = session_file(&self.root, &path) else { continue };
                let Ok(md) = e.metadata() else { continue };
                if md.modified().map(|m| m < cutoff).unwrap_or(true) {
                    continue;
                }
                let size = md.len();
                let mtime = mtime_ms(&md);
                let (mut meta, mut offset) = match cached.remove(&path) {
                    Some(c) if c.offset <= size => (c.meta, c.offset),
                    _ => (Meta::default(), 0),
                };
                if offset < size {
                    let mut sink = Vec::new();
                    match read_lines_from(&path, offset, None, &mut meta, &mut sink) {
                        Ok(o) => offset = o,
                        Err(_) => continue,
                    }
                    self.dirty = true;
                }
                if meta.last_ts == 0 {
                    meta.last_ts = mtime;
                }
                bulk.push((sid.clone(), meta.clone(), mtime));
                self.files.insert(sid, FileState { path, offset, meta, mtime });
            }
        }
        let _ = self.out.send(TxOut::Bulk(bulk));
        self.save_cache();
    }

    fn save_cache(&mut self) {
        if !self.dirty {
            return;
        }
        let entries: Vec<CacheEntry> = self
            .files
            .values()
            .map(|f| CacheEntry { path: f.path.clone(), size: f.offset, offset: f.offset, meta: f.meta.clone() })
            .collect();
        if let Ok(bytes) = serde_json::to_vec(&entries) {
            let tmp = self.cache_path.with_extension("tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &self.cache_path);
            }
        }
        self.dirty = false;
    }

    fn run(&mut self, rx: Receiver<TxCmd>) {
        let mut last_save = Instant::now();
        let mut changed: HashSet<PathBuf> = HashSet::new();
        loop {
            let first = match rx.recv_timeout(Duration::from_secs(30)) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let mut cmds: Vec<TxCmd> = first.into_iter().collect();
            if !cmds.is_empty() {
                std::thread::sleep(Duration::from_millis(15));
                while let Ok(c) = rx.try_recv() {
                    cmds.push(c);
                }
            }
            for cmd in cmds {
                match cmd {
                    TxCmd::Fs(Ok(ev)) => {
                        for p in ev.paths {
                            if session_file(&self.root, &p).is_some() {
                                changed.insert(p);
                            }
                        }
                    }
                    TxCmd::Fs(Err(_)) => {}
                    TxCmd::Track(p) => {
                        if session_file(&self.root, &p).is_some() {
                            changed.insert(p);
                        }
                    }
                    TxCmd::Load { sid, reply } => {
                        for p in changed.drain().collect::<Vec<_>>() {
                            self.process(&p);
                        }
                        self.load(sid, reply);
                    }
                }
            }
            for p in changed.drain().collect::<Vec<_>>() {
                self.process(&p);
            }
            if last_save.elapsed() > Duration::from_secs(60) {
                self.save_cache();
                last_save = Instant::now();
            }
        }
    }

    fn process(&mut self, path: &Path) {
        let Some(sid) = session_file(&self.root, path) else { return };
        let Ok(md) = std::fs::metadata(path) else { return };
        let size = md.len();
        let mtime = mtime_ms(&md);
        let state = self.files.entry(sid.clone()).or_insert_with(|| FileState {
            path: path.to_path_buf(),
            offset: 0,
            meta: Meta::default(),
            mtime,
        });
        if size < state.offset {
            state.offset = 0;
            state.meta = Meta::default();
        }
        if size == state.offset {
            return;
        }
        let mut items = Vec::new();
        match read_lines_from(path, state.offset, None, &mut state.meta, &mut items) {
            Ok(o) => state.offset = o,
            Err(_) => return,
        }
        state.mtime = mtime;
        if state.meta.last_ts == 0 {
            state.meta.last_ts = mtime;
        }
        self.dirty = true;
        let meta = state.meta.clone();
        if !items.is_empty() {
            let _ = self.out.send(TxOut::Items(sid.clone(), items));
        }
        let _ = self.out.send(TxOut::Meta(sid, meta, mtime));
    }

    fn load(&mut self, sid: String, reply: oneshot::Sender<Vec<Item>>) {
        let mut items = Vec::new();
        if let Some(state) = self.files.get(&sid) {
            let mut meta = Meta::default();
            let _ = read_lines_from(&state.path, 0, Some(state.offset), &mut meta, &mut items);
        }
        let _ = self.out.send(TxOut::Loaded(sid, items, reply));
    }
}
