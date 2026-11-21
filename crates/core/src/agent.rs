use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};
use serde_json::{json, Map, Value};
use tokio::sync::{broadcast, mpsc, oneshot, Notify};

use crate::config::{Config, Paths};
use crate::model::*;
use crate::transcript::{Meta, TxCmd, TxOut};
use crate::{install, runner, scheduler};

pub const STOP_CHAIN_LIMIT: u32 = 7;
const OPEN_CACHE: usize = 16;
const NOTES_MAX: usize = 60;
const INJECT_PREFIX: &str = "[Berklin] Mensagem do usuário enviada pelo celular:";

pub struct HookState {
    pub live: bool,
    pub status: Status,
    pub stop_chain: u32,
    pub queue: VecDeque<String>,
}

pub struct Run {
    pub stdin: mpsc::UnboundedSender<String>,
    pub kill: Option<oneshot::Sender<()>>,
    pub schedule: Option<String>,
    pub busy: bool,
}

struct Conn {
    local: Option<mpsc::UnboundedSender<Arc<str>>>,
    remote: Option<String>,
    subs: HashSet<String>,
}

pub struct Inner {
    pub sessions: HashMap<String, Session>,
    last_kind: HashMap<String, String>,
    pub hooks: HashMap<String, HookState>,
    open: VecDeque<(String, Vec<Item>)>,
    pending: HashMap<String, (Pending, Option<oneshot::Sender<Resolution>>)>,
    notes: VecDeque<Note>,
    pub schedules: Vec<Schedule>,
    away: Away,
    away_since: Instant,
    conns: HashMap<u64, Conn>,
    pub runs: HashMap<String, Run>,
    pub chrome_sids: HashSet<String>,
}

pub struct Agent {
    pub paths: Paths,
    pub cfg: RwLock<Config>,
    pub inner: Mutex<Inner>,
    tx_cmd: Mutex<std::sync::mpsc::Sender<TxCmd>>,
    relay_out: mpsc::UnboundedSender<(Option<String>, Arc<str>)>,
    pub alerts: broadcast::Sender<Alert>,
    pub sched_wake: Notify,
    pub relay_wake: Notify,
    pub hook_exe: PathBuf,
    next_conn: AtomicU64,
    pub relay_online: AtomicBool,
    pub update: Mutex<crate::update::UpdateState>,
    pub dashboards: Mutex<HashMap<String, crate::dashboard::Dashboard>>,
    pub update_wake: Notify,
    pub restart: Notify,
}

fn ev(name: &str, mut body: Value) -> Arc<str> {
    if let Value::Object(m) = &mut body {
        m.insert("ev".into(), Value::String(name.into()));
    }
    body.to_string().into()
}

fn basename(p: &str) -> String {
    Path::new(p).file_name().and_then(|s| s.to_str()).unwrap_or(p).to_string()
}

fn short_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

impl Agent {
    pub fn new(
        paths: Paths,
        cfg: Config,
        tx_cmd: std::sync::mpsc::Sender<TxCmd>,
        relay_out: mpsc::UnboundedSender<(Option<String>, Arc<str>)>,
        hook_exe: PathBuf,
    ) -> Arc<Agent> {
        let schedules: Vec<Schedule> = std::fs::read(paths.schedules())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let (alerts, _) = broadcast::channel(64);
        let dashboards = crate::dashboard::load_all(&paths.dir);
        Arc::new(Agent {
            paths,
            cfg: RwLock::new(cfg),
            inner: Mutex::new(Inner {
                sessions: HashMap::new(),
                last_kind: HashMap::new(),
                hooks: HashMap::new(),
                open: VecDeque::new(),
                pending: HashMap::new(),
                notes: VecDeque::new(),
                schedules,
                away: Away::default(),
                away_since: Instant::now(),
                conns: HashMap::new(),
                runs: HashMap::new(),
                chrome_sids: HashSet::new(),
            }),
            tx_cmd: Mutex::new(tx_cmd),
            relay_out,
            alerts,
            sched_wake: Notify::new(),
            relay_wake: Notify::new(),
            hook_exe,
            next_conn: AtomicU64::new(1),
            relay_online: AtomicBool::new(false),
            update: Mutex::new(crate::update::UpdateState::default()),
            dashboards: Mutex::new(dashboards),
            update_wake: Notify::new(),
            restart: Notify::new(),
        })
    }

    pub fn pending_count(&self, inner: &Inner) -> usize {
        inner.pending.len()
    }

    pub fn log(&self, msg: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.paths.log()) {
            let _ = writeln!(f, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
        }
    }

    fn emit_all(&self, inner: &Inner, msg: Arc<str>) {
        let mut remote = false;
        for c in inner.conns.values() {
            if let Some(tx) = &c.local {
                let _ = tx.send(msg.clone());
            }
            remote |= c.remote.is_some();
        }
        if remote {
            let _ = self.relay_out.send((None, msg));
        }
    }

    fn emit_sub(&self, inner: &Inner, sid: &str, msg: Arc<str>) {
        let mut remote_total = 0;
        let mut remote_subs: Vec<&String> = Vec::new();
        for c in inner.conns.values() {
            let sub = c.subs.contains(sid);
            if let Some(dev) = &c.remote {
                remote_total += 1;
                if sub {
                    remote_subs.push(dev);
                }
            } else if sub {
                if let Some(tx) = &c.local {
                    let _ = tx.send(msg.clone());
                }
            }
        }
        if !remote_subs.is_empty() && remote_subs.len() == remote_total {
            let _ = self.relay_out.send((None, msg));
        } else {
            for dev in remote_subs {
                let _ = self.relay_out.send((Some(dev.clone()), msg.clone()));
            }
        }
    }

    fn send_to(&self, conn: u64, msg: Arc<str>) {
        let inner = self.inner.lock();
        if let Some(c) = inner.conns.get(&conn) {
            if let Some(tx) = &c.local {
                let _ = tx.send(msg);
            } else if let Some(dev) = &c.remote {
                let _ = self.relay_out.send((Some(dev.clone()), msg));
            }
        }
    }

    pub fn add_local_conn(&self, tx: mpsc::UnboundedSender<Arc<str>>) -> u64 {
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        self.inner.lock().conns.insert(id, Conn { local: Some(tx), remote: None, subs: HashSet::new() });
        id
    }

    pub fn remote_conn(&self, dev: &str) -> u64 {
        let mut inner = self.inner.lock();
        if let Some((id, _)) = inner.conns.iter().find(|(_, c)| c.remote.as_deref() == Some(dev)) {
            return *id;
        }
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        inner.conns.insert(id, Conn { local: None, remote: Some(dev.to_string()), subs: HashSet::new() });
        id
    }

    pub fn drop_conn(&self, id: u64) {
        self.inner.lock().conns.remove(&id);
    }

    pub fn drop_remote(&self, dev: Option<&str>) {
        self.inner.lock().conns.retain(|_, c| match (&c.remote, dev) {
            (Some(_), None) => false,
            (Some(d), Some(x)) => d != x,
            _ => true,
        });
    }

    fn compute_status(inner: &Inner, sid: &str, updated: i64) -> (Status, bool, bool) {
        let headless = inner.runs.contains_key(sid);
        let hook = inner.hooks.get(sid);
        let live = headless || hook.map(|h| h.live).unwrap_or(false);
        if inner.pending.values().any(|(p, _)| p.sid == sid) {
            return (Status::Waiting, live, headless);
        }
        if let Some(r) = inner.runs.get(sid) {
            return (if r.busy { Status::Running } else { Status::Idle }, live, headless);
        }
        if let Some(h) = hook {
            if h.live {
                return (h.status, live, headless);
            }
        }
        let kind = inner.last_kind.get(sid).map(String::as_str).unwrap_or("text");
        let fresh = now_ms() - updated < 45_000;
        let status = if fresh && kind != "text" && kind != "info" { Status::Running } else { Status::Idle };
        (status, live, headless)
    }

    pub fn refresh(&self, inner: &mut Inner, sid: &str) {
        let Some(updated) = inner.sessions.get(sid).map(|s| s.updated) else { return };
        let (status, live, headless) = Self::compute_status(inner, sid, updated);
        let s = inner.sessions.get_mut(sid).unwrap();
        if s.status != status || s.live != live || s.headless != headless {
            s.status = status;
            s.live = live;
            s.headless = headless;
            let msg = ev("session", json!({ "s": s }));
            self.emit_all(inner, msg);
        }
    }

    fn apply_meta(&self, inner: &mut Inner, sid: &str, meta: &Meta, mtime: i64) {
        let s = inner.sessions.entry(sid.to_string()).or_default();
        s.sid = sid.to_string();
        s.title = meta.title();
        s.cwd = meta.cwd.clone();
        s.project = basename(&meta.cwd);
        s.updated = meta.last_ts.max(if meta.last_ts == 0 { mtime } else { 0 });
        s.preview = meta.preview.clone();
        s.model = meta.model.clone();
        s.tokens = meta.tokens;
        let updated = s.updated;
        inner.last_kind.insert(sid.to_string(), meta.last_kind.clone());
        let (status, live, headless) = Self::compute_status(inner, sid, updated);
        let s = inner.sessions.get_mut(sid).unwrap();
        s.status = status;
        s.live = live;
        s.headless = headless;
    }

    pub fn on_tx(&self, out: TxOut) {
        let mut inner = self.inner.lock();
        match out {
            TxOut::Bulk(list) => {
                for (sid, meta, mtime) in list {
                    self.apply_meta(&mut inner, &sid, &meta, mtime);
                }
            }
            TxOut::Meta(sid, meta, mtime) => {
                self.apply_meta(&mut inner, &sid, &meta, mtime);
                let msg = ev("session", json!({ "s": inner.sessions.get(&sid) }));
                self.emit_all(&inner, msg);
            }
            TxOut::Items(sid, items) => {
                if let Some((_, cached)) = inner.open.iter_mut().find(|(s, _)| *s == sid) {
                    cached.extend(items.iter().cloned());
                    let msg = ev("items", json!({ "sid": sid, "items": items }));
                    self.emit_sub(&inner, &sid, msg);
                }
            }
            TxOut::Loaded(sid, items, reply) => {
                if !inner.open.iter().any(|(s, _)| *s == sid) {
                    inner.open.push_front((sid, items));
                    while inner.open.len() > OPEN_CACHE {
                        let subscribed: HashSet<String> = inner.conns.values().flat_map(|c| c.subs.iter().cloned()).collect();
                        if let Some(pos) = inner.open.iter().rposition(|(s, _)| !subscribed.contains(s)) {
                            inner.open.remove(pos);
                        } else {
                            break;
                        }
                    }
                }
                let _ = reply.send(Vec::new());
            }
        }
    }

    pub fn tick_status(&self) {
        let mut inner = self.inner.lock();
        let running: Vec<String> = inner
            .sessions
            .values()
            .filter(|s| s.status == Status::Running && !s.live)
            .map(|s| s.sid.clone())
            .collect();
        for sid in running {
            self.refresh(&mut inner, &sid);
        }
    }

    pub fn track_transcript(&self, path: &str) {
        let _ = self.tx_cmd.lock().send(TxCmd::Track(PathBuf::from(path)));
    }

    pub fn note(&self, kind: &'static str, title: String, body: String, sid: Option<String>, pid: Option<String>) {
        let n = Note { nid: short_id(), ts: now_ms(), kind, title: title.clone(), body: body.clone(), sid: sid.clone(), pid };
        let mut inner = self.inner.lock();
        inner.notes.push_back(n.clone());
        while inner.notes.len() > NOTES_MAX {
            inner.notes.pop_front();
        }
        let msg = ev("note", json!({ "n": n }));
        self.emit_all(&inner, msg);
        drop(inner);
        let _ = self.alerts.send(Alert { title, body, sid });
    }

    pub fn session_title(&self, sid: &str) -> String {
        self.inner.lock().sessions.get(sid).map(|s| s.title.clone()).unwrap_or_else(|| "Claude".into())
    }

    pub fn away(&self) -> Away {
        self.inner.lock().away
    }

    pub fn away_since(&self) -> Instant {
        self.inner.lock().away_since
    }

    pub fn set_away(&self, on: bool, auto: bool) {
        let mut inner = self.inner.lock();
        if inner.away.on == on {
            return;
        }
        inner.away = Away { on, auto };
        inner.away_since = Instant::now();
        let msg = ev("away", json!({ "a": inner.away }));
        self.emit_all(&inner, msg);
        if !on {
            let release: Vec<String> = inner
                .pending
                .values()
                .filter(|(p, _)| !inner.runs.contains_key(&p.sid))
                .map(|(p, _)| p.pid.clone())
                .collect();
            drop(inner);
            for pid in release {
                self.resolve(&pid, Resolution::Release);
            }
        }
    }

    pub fn resolve(&self, pid: &str, res: Resolution) -> bool {
        let mut inner = self.inner.lock();
        let Some((p, tx)) = inner.pending.remove(pid) else { return false };
        if let Some(tx) = tx {
            let _ = tx.send(res);
        }
        let msg = ev("pending.done", json!({ "pid": pid }));
        self.emit_all(&inner, msg);
        self.refresh(&mut inner, &p.sid);
        true
    }

    pub async fn wait_pending(self: &Arc<Self>, p: Pending, alert_title: String, alert_body: String) -> Resolution {
        let (tx, rx) = oneshot::channel();
        let pid = p.pid.clone();
        let sid = p.sid.clone();
        {
            let mut inner = self.inner.lock();
            let msg = ev("pending", json!({ "p": &p }));
            inner.pending.insert(pid.clone(), (p, Some(tx)));
            self.emit_all(&inner, msg);
            self.refresh(&mut inner, &sid);
        }
        let _ = self.alerts.send(Alert { title: alert_title, body: alert_body, sid: Some(sid) });
        struct Guard(Arc<Agent>, String);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.resolve(&self.1, Resolution::Release);
            }
        }
        let _guard = Guard(self.clone(), pid);
        rx.await.unwrap_or(Resolution::Release)
    }

    fn should_route(&self, sid: &str, headless_hdr: bool) -> bool {
        let inner = self.inner.lock();
        inner.away.on || headless_hdr || inner.runs.contains_key(sid)
    }

    fn hook_state<'a>(inner: &'a mut Inner, sid: &str) -> &'a mut HookState {
        inner
            .hooks
            .entry(sid.to_string())
            .or_insert_with(|| HookState { live: true, status: Status::Idle, stop_chain: 0, queue: VecDeque::new() })
    }

    fn ensure_session(&self, inner: &mut Inner, sid: &str, cwd: &str) {
        if !inner.sessions.contains_key(sid) {
            inner.sessions.insert(
                sid.to_string(),
                Session {
                    sid: sid.to_string(),
                    title: "Nova sessão".into(),
                    cwd: cwd.to_string(),
                    project: basename(cwd),
                    updated: now_ms(),
                    ..Default::default()
                },
            );
        }
    }

    pub async fn hook(self: &Arc<Self>, event: &str, body: Value, headless_hdr: bool) -> Value {
        let sid = body.get("session_id").and_then(Value::as_str).unwrap_or("").to_string();
        let cwd = body.get("cwd").and_then(Value::as_str).unwrap_or("").to_string();
        if sid.is_empty() {
            return Value::Null;
        }
        if let Some(tp) = body.get("transcript_path").and_then(Value::as_str) {
            self.track_transcript(tp);
        }
        {
            let mut inner = self.inner.lock();
            self.ensure_session(&mut inner, &sid, &cwd);
            let h = Self::hook_state(&mut inner, &sid);
            match event {
                "SessionStart" => {
                    h.live = true;
                    h.status = Status::Idle;
                    h.stop_chain = 0;
                }
                "SessionEnd" => {
                    h.live = false;
                    h.status = Status::Idle;
                }
                "UserPromptSubmit" => {
                    h.live = true;
                    h.status = Status::Running;
                    h.stop_chain = 0;
                }
                "PreToolUse" | "PermissionRequest" => {
                    h.live = true;
                    h.status = Status::Running;
                }
                _ => {}
            }
            self.refresh(&mut inner, &sid);
        }
        match event {
            "SessionEnd" => {
                let pids: Vec<String> = {
                    let inner = self.inner.lock();
                    inner.pending.values().filter(|(p, _)| p.sid == sid).map(|(p, _)| p.pid.clone()).collect()
                };
                for pid in pids {
                    self.resolve(&pid, Resolution::Release);
                }
                Value::Null
            }
            "PreToolUse" => self.hook_question(&sid, &cwd, &body, headless_hdr).await,
            "PermissionRequest" => self.hook_permission(&sid, &cwd, &body, headless_hdr).await,
            "Notification" => {
                self.hook_notification(&sid, &body);
                Value::Null
            }
            "Stop" => self.hook_stop(&sid, &cwd, &body, headless_hdr).await,
            _ => Value::Null,
        }
    }

    async fn hook_question(self: &Arc<Self>, sid: &str, cwd: &str, body: &Value, headless_hdr: bool) -> Value {
        if body.get("tool_name").and_then(Value::as_str) != Some("AskUserQuestion") {
            return Value::Null;
        }
        let input = body.get("tool_input").cloned().unwrap_or(Value::Null);
        let questions = input.get("questions").cloned().unwrap_or(Value::Array(vec![]));
        let title = self.session_title(sid);
        let first_q = questions
            .get(0)
            .and_then(|q| q.get("question"))
            .and_then(Value::as_str)
            .unwrap_or("Claude tem uma pergunta")
            .to_string();
        if !self.should_route(sid, headless_hdr) {
            self.note("needs", format!("{title}: pergunta"), first_q, Some(sid.to_string()), None);
            return Value::Null;
        }
        let p = Pending {
            pid: short_id(),
            sid: sid.to_string(),
            kind: PendingKind::Question,
            ts: now_ms(),
            title: title.clone(),
            cwd: cwd.to_string(),
            tool: Some("AskUserQuestion".into()),
            input: None,
            questions: Some(questions),
            message: None,
        };
        match self.wait_pending(p, format!("{title}: pergunta"), first_q).await {
            Resolution::Answers(answers) => {
                let mut updated = input.as_object().cloned().unwrap_or_default();
                updated.insert("answers".into(), Value::Object(answers));
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "permissionDecisionReason": "Respondido pelo Berklin",
                        "updatedInput": Value::Object(updated)
                    }
                })
            }
            Resolution::Text(t) => {
                let mut answers = Map::new();
                let q = input
                    .get("questions")
                    .and_then(|q| q.get(0))
                    .and_then(|q| q.get("question"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                answers.insert(q, Value::String(t));
                let mut updated = input.as_object().cloned().unwrap_or_default();
                updated.insert("answers".into(), Value::Object(answers));
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "updatedInput": Value::Object(updated)
                    }
                })
            }
            Resolution::Deny(m) => json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": m.unwrap_or_else(|| "O usuário recusou responder.".into())
                }
            }),
            _ => Value::Null,
        }
    }

    async fn hook_permission(self: &Arc<Self>, sid: &str, cwd: &str, body: &Value, headless_hdr: bool) -> Value {
        if !self.should_route(sid, headless_hdr) {
            return Value::Null;
        }
        let tool = body.get("tool_name").and_then(Value::as_str).unwrap_or("ferramenta").to_string();
        let input = body.get("tool_input").cloned().unwrap_or(Value::Null);
        let title = self.session_title(sid);
        let summary = describe_tool(&tool, &input);
        let p = Pending {
            pid: short_id(),
            sid: sid.to_string(),
            kind: PendingKind::Permission,
            ts: now_ms(),
            title: title.clone(),
            cwd: cwd.to_string(),
            tool: Some(tool.clone()),
            input: Some(input),
            questions: None,
            message: None,
        };
        match self.wait_pending(p, format!("{title}: permissão para {tool}"), summary).await {
            Resolution::Allow(_) => json!({
                "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": { "behavior": "allow" } }
            }),
            Resolution::Deny(m) => json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": { "behavior": "deny", "message": m.unwrap_or_else(|| "Negado pelo usuário no Berklin.".into()) }
                }
            }),
            Resolution::Text(t) => json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": { "behavior": "deny", "message": t }
                }
            }),
            _ => Value::Null,
        }
    }

    fn hook_notification(&self, sid: &str, body: &Value) {
        let kind = body.get("notification_type").and_then(Value::as_str).unwrap_or("");
        let has_pending = self.inner.lock().pending.values().any(|(p, _)| p.sid == sid);
        if has_pending {
            return;
        }
        let message = body.get("message").and_then(Value::as_str).unwrap_or("").to_string();
        let title = self.session_title(sid);
        match kind {
            "permission_prompt" => self.note("needs", format!("{title}: precisa de permissão"), message, Some(sid.into()), None),
            "elicitation_dialog" | "agent_needs_input" | "elicitation_url_dialog" => {
                self.note("needs", format!("{title}: precisa de você"), message, Some(sid.into()), None)
            }
            "idle_prompt" => {
                if self.away().on {
                    self.note("needs", format!("{title}: esperando você"), message, Some(sid.into()), None)
                }
            }
            _ => {}
        }
    }

    async fn hook_stop(self: &Arc<Self>, sid: &str, cwd: &str, body: &Value, headless_hdr: bool) -> Value {
        if headless_hdr || self.inner.lock().runs.contains_key(sid) {
            return Value::Null;
        }
        let last = body.get("last_assistant_message").and_then(Value::as_str).unwrap_or("").to_string();
        let (queued, chain, away) = {
            let mut inner = self.inner.lock();
            let away = inner.away.on;
            let h = Self::hook_state(&mut inner, sid);
            let chain = h.stop_chain;
            let q = if chain < STOP_CHAIN_LIMIT { h.queue.pop_front() } else { None };
            if q.is_none() {
                h.status = Status::Idle;
            }
            self.refresh(&mut inner, sid);
            (q, chain, away)
        };
        if let Some(text) = queued {
            return self.continue_with(sid, text);
        }
        let title = self.session_title(sid);
        if !away {
            return Value::Null;
        }
        if chain >= STOP_CHAIN_LIMIT {
            self.note(
                "done",
                format!("{title} terminou"),
                format!("{}\n\nLimite de continuações seguidas atingido; a próxima mensagem retoma em segundo plano.", one_line(&last, 300)),
                Some(sid.to_string()),
                None,
            );
            return Value::Null;
        }
        let p = Pending {
            pid: short_id(),
            sid: sid.to_string(),
            kind: PendingKind::Stop,
            ts: now_ms(),
            title: title.clone(),
            cwd: cwd.to_string(),
            tool: None,
            input: None,
            questions: None,
            message: Some(clip(&last, 4000)),
        };
        match self.wait_pending(p, format!("{title} terminou"), one_line(&last, 200)).await {
            Resolution::Text(t) => self.continue_with(sid, t),
            _ => Value::Null,
        }
    }

    fn continue_with(&self, sid: &str, text: String) -> Value {
        let mut inner = self.inner.lock();
        let h = Self::hook_state(&mut inner, sid);
        h.stop_chain += 1;
        h.status = Status::Running;
        self.refresh(&mut inner, sid);
        json!({ "decision": "block", "reason": format!("{INJECT_PREFIX}\n\n{text}") })
    }

    pub fn snapshot(&self) -> Value {
        let inner = self.inner.lock();
        let mut sessions: Vec<&Session> = inner.sessions.values().collect();
        sessions.sort_unstable_by(|a, b| b.updated.cmp(&a.updated));
        sessions.truncate(150);
        let pending: Vec<&Pending> = inner.pending.values().map(|(p, _)| p).collect();
        json!({
            "pc": { "name": self.cfg.read().pc_name, "version": crate::update::VERSION },
            "away": inner.away,
            "sessions": sessions,
            "pending": pending,
            "schedules": inner.schedules,
            "notes": inner.notes,
            "dashboards": self.dashboard_metas(),
        })
    }

    pub fn dashboard_metas(&self) -> Vec<Value> {
        let map = self.dashboards.lock();
        let mut list: Vec<&crate::dashboard::Dashboard> = map.values().collect();
        list.sort_unstable_by(|a, b| b.ts.cmp(&a.ts));
        list.iter().map(|d| d.meta()).collect()
    }

    pub fn remove_dashboard(&self, key: &str) {
        if self.dashboards.lock().remove(key).is_some() {
            crate::dashboard::remove(&self.paths.dir, key);
            let inner = self.inner.lock();
            let msg = ev("dashboard.gone", json!({ "key": key }));
            self.emit_all(&inner, msg);
        }
    }

    fn publish_dashboard(&self, sid: Option<String>, schedule: Option<String>, args: &Value) -> Result<String, String> {
        let (kind, content, title) = crate::dashboard::content_from_args(args)?;
        let sched_name = schedule
            .as_ref()
            .and_then(|id| self.inner.lock().schedules.iter().find(|s| &s.id == id).map(|s| s.name.clone()));
        let key = match (&schedule, &sid) {
            (Some(id), _) => id.clone(),
            (None, Some(s)) => format!("s-{s}"),
            (None, None) => "geral".to_string(),
        };
        let name = sched_name.unwrap_or_else(|| sid.as_deref().map(|x| self.session_title(x)).unwrap_or_else(|| "Claude".into()));
        let d = crate::dashboard::Dashboard {
            key: key.clone(),
            title: if title.is_empty() { name.clone() } else { title },
            name,
            ts: now_ms(),
            kind,
            sched: schedule,
            sid,
            content,
        };
        crate::dashboard::save(&self.paths.dir, &d)?;
        let meta = d.meta();
        self.dashboards.lock().insert(key, d);
        let inner = self.inner.lock();
        let msg = ev("dashboard", json!({ "d": meta }));
        self.emit_all(&inner, msg);
        Ok("Painel publicado no app Berklin.".into())
    }

    pub async fn handle_text(self: &Arc<Self>, conn: u64, text: &str) {
        let Ok(v) = serde_json::from_str::<Value>(text) else { return };
        let id = v.get("id").cloned().unwrap_or(Value::Null);
        let op = v.get("op").and_then(Value::as_str).unwrap_or("").to_string();
        let res = self.op(conn, &op, &v).await;
        let msg = match res {
            Ok(data) => json!({ "re": id, "ok": true, "data": data }),
            Err(e) => json!({ "re": id, "ok": false, "err": e }),
        };
        self.send_to(conn, msg.to_string().into());
    }

    fn is_local(&self, conn: u64) -> bool {
        self.inner.lock().conns.get(&conn).map(|c| c.local.is_some()).unwrap_or(false)
    }

    async fn ensure_loaded(&self, sid: &str) {
        if self.inner.lock().open.iter().any(|(s, _)| s == sid) {
            return;
        }
        let (tx, rx) = oneshot::channel();
        let _ = self.tx_cmd.lock().send(TxCmd::Load { sid: sid.to_string(), reply: tx });
        let _ = rx.await;
    }

    async fn op(self: &Arc<Self>, conn: u64, op: &str, v: &Value) -> Result<Value, String> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        match op {
            "hello" => Ok(self.snapshot()),
            "sessions.list" => {
                let limit = v.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
                let before = v.get("before").and_then(Value::as_i64).unwrap_or(i64::MAX);
                let inner = self.inner.lock();
                let mut list: Vec<&Session> = inner.sessions.values().filter(|s| s.updated < before).collect();
                list.sort_unstable_by(|a, b| b.updated.cmp(&a.updated));
                list.truncate(limit);
                Ok(json!(list))
            }
            "session.open" => {
                let sid = s("sid").ok_or("sid")?;
                let limit = v.get("limit").and_then(Value::as_u64).unwrap_or(150) as usize;
                self.ensure_loaded(&sid).await;
                let mut inner = self.inner.lock();
                if let Some(c) = inner.conns.get_mut(&conn) {
                    c.subs.insert(sid.clone());
                }
                let pos = inner.open.iter().position(|(x, _)| *x == sid);
                let Some(pos) = pos else { return Ok(json!({ "sid": sid, "items": [], "more": false })) };
                let entry = inner.open.remove(pos).unwrap();
                let start = entry.1.len().saturating_sub(limit);
                let out = json!({ "sid": sid, "items": &entry.1[start..], "more": start > 0 });
                inner.open.push_front(entry);
                Ok(out)
            }
            "session.older" => {
                let sid = s("sid").ok_or("sid")?;
                let before = v.get("before").and_then(Value::as_u64).unwrap_or(0) as usize;
                let limit = v.get("limit").and_then(Value::as_u64).unwrap_or(150) as usize;
                self.ensure_loaded(&sid).await;
                let inner = self.inner.lock();
                let Some((_, items)) = inner.open.iter().find(|(x, _)| *x == sid) else {
                    return Ok(json!({ "items": [], "more": false }));
                };
                let end = items.iter().position(|it| it.i as usize >= before).unwrap_or(items.len());
                let start = end.saturating_sub(limit);
                Ok(json!({ "items": &items[start..end], "more": start > 0 }))
            }
            "session.close" => {
                let sid = s("sid").ok_or("sid")?;
                if let Some(c) = self.inner.lock().conns.get_mut(&conn) {
                    c.subs.remove(&sid);
                }
                Ok(Value::Null)
            }
            "send" => self.op_send(v).await,
            "interrupt" => {
                let sid = s("sid").ok_or("sid")?;
                let kill = self.inner.lock().runs.get_mut(&sid).and_then(|r| r.kill.take());
                match kill {
                    Some(k) => {
                        let _ = k.send(());
                        Ok(Value::Null)
                    }
                    None => Err("Essa sessão não está rodando pelo Berklin.".into()),
                }
            }
            "pending.resolve" => {
                let pid = s("pid").ok_or("pid")?;
                let res = if v.get("release").and_then(Value::as_bool) == Some(true) {
                    Resolution::Release
                } else if let Some(a) = v.get("answers").and_then(Value::as_object) {
                    Resolution::Answers(a.clone())
                } else if let Some(t) = s("text") {
                    Resolution::Text(t)
                } else if let Some(allow) = v.get("allow").and_then(Value::as_bool) {
                    if allow {
                        Resolution::Allow(s("message"))
                    } else {
                        Resolution::Deny(s("message"))
                    }
                } else {
                    return Err("resposta vazia".into());
                };
                if self.resolve(&pid, res) {
                    Ok(Value::Null)
                } else {
                    Err("Esse pedido já foi respondido.".into())
                }
            }
            "away.set" => {
                let on = v.get("on").and_then(Value::as_bool).unwrap_or(false);
                self.set_away(on, false);
                Ok(json!(self.away()))
            }
            "schedules.save" => {
                let sched: Schedule = serde_json::from_value(v.get("schedule").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
                scheduler::save(self, sched).map(|s| json!(s))
            }
            "schedules.delete" => {
                let id = s("sched").ok_or("sched")?;
                scheduler::delete(self, &id);
                self.remove_dashboard(&id);
                Ok(Value::Null)
            }
            "dashboards.list" => Ok(json!(self.dashboard_metas())),
            "dashboard.get" => {
                let key = s("key").ok_or("key")?;
                self.dashboards.lock().get(&key).map(|d| json!(d)).ok_or_else(|| "Painel não encontrado.".to_string())
            }
            "dashboard.delete" => {
                let key = s("key").ok_or("key")?;
                self.remove_dashboard(&key);
                Ok(Value::Null)
            }
            "schedules.run" => {
                let id = s("sched").ok_or("sched")?;
                scheduler::run_now(self, &id).await.map(|_| Value::Null)
            }
            "projects.list" => {
                let inner = self.inner.lock();
                let mut by_cwd: HashMap<&str, i64> = HashMap::new();
                for s in inner.sessions.values() {
                    if s.cwd.is_empty() {
                        continue;
                    }
                    let e = by_cwd.entry(s.cwd.as_str()).or_insert(0);
                    *e = (*e).max(s.updated);
                }
                let mut list: Vec<Value> = by_cwd
                    .into_iter()
                    .filter(|(cwd, _)| Path::new(cwd).is_dir())
                    .map(|(cwd, last)| json!({ "cwd": cwd, "name": basename(cwd), "last": last }))
                    .collect();
                list.sort_unstable_by(|a, b| b["last"].as_i64().cmp(&a["last"].as_i64()));
                Ok(json!(list))
            }
            "notes.ack" => {
                let ids: HashSet<String> = v
                    .get("ids")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                self.inner.lock().notes.retain(|n| !ids.contains(&n.nid));
                Ok(Value::Null)
            }
            "config.get" if self.is_local(conn) => Ok(self.config_view()),
            "config.set" if self.is_local(conn) => {
                let patch = v.get("config").cloned().unwrap_or(Value::Null);
                self.config_set(patch)?;
                Ok(self.config_view())
            }
            "pair.rotate" if self.is_local(conn) => {
                {
                    let mut cfg = self.cfg.write();
                    cfg.key = crate::config::random_b64(32);
                    cfg.save(&self.paths);
                }
                self.relay_wake.notify_one();
                Ok(self.config_view())
            }
            "integration.install" if self.is_local(conn) => {
                let agent = self.clone();
                tokio::task::spawn_blocking(move || install::install_all(&agent))
                    .await
                    .map_err(|e| e.to_string())??;
                Ok(self.config_view())
            }
            "integration.remove" if self.is_local(conn) => {
                let agent = self.clone();
                tokio::task::spawn_blocking(move || install::remove_all(&agent))
                    .await
                    .map_err(|e| e.to_string())??;
                Ok(self.config_view())
            }
            "power.get" if self.is_local(conn) => {
                let lid = tokio::task::spawn_blocking(crate::power::lid_actions).await.ok().flatten();
                Ok(json!({
                    "keepAwake": self.cfg.read().keep_awake,
                    "lidAc": lid.map(|l| l.0),
                    "lidDc": lid.map(|l| l.1),
                }))
            }
            "power.lid" if self.is_local(conn) => {
                let awake = v.get("awake").and_then(Value::as_bool).unwrap_or(false);
                tokio::task::spawn_blocking(move || crate::power::set_lid_stay_awake_on_ac(awake))
                    .await
                    .map_err(|e| e.to_string())??;
                let lid = tokio::task::spawn_blocking(crate::power::lid_actions).await.ok().flatten();
                Ok(json!({ "keepAwake": self.cfg.read().keep_awake, "lidAc": lid.map(|l| l.0), "lidDc": lid.map(|l| l.1) }))
            }
            "update.check" if self.is_local(conn) => {
                let st = crate::update::check(self).await;
                Ok(json!(st))
            }
            "update.apply" if self.is_local(conn) => {
                crate::update::apply(self)?;
                Ok(Value::Null)
            }
            _ => Err(format!("operação desconhecida: {op}")),
        }
    }

    async fn op_send(self: &Arc<Self>, v: &Value) -> Result<Value, String> {
        let text = v.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if text.is_empty() {
            return Err("Mensagem vazia.".into());
        }
        let mode = v
            .get("mode")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| self.cfg.read().default_mode.clone());
        let Some(sid) = v.get("sid").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string) else {
            let cwd = v.get("cwd").and_then(Value::as_str).unwrap_or("").to_string();
            if cwd.is_empty() || !Path::new(&cwd).is_dir() {
                return Err("Escolha uma pasta válida para o novo chat.".into());
            }
            let chrome = v.get("chrome").and_then(Value::as_bool).unwrap_or(false);
            let sid = runner::start(self, None, cwd, text, mode, None, chrome).await?;
            return Ok(json!({ "sid": sid, "how": "started" }));
        };
        let stop_pid = {
            let inner = self.inner.lock();
            inner.pending.values().find(|(p, _)| p.sid == sid && p.kind == PendingKind::Stop).map(|(p, _)| p.pid.clone())
        };
        if let Some(pid) = stop_pid {
            self.resolve(&pid, Resolution::Text(text));
            return Ok(json!({ "sid": sid, "how": "injected" }));
        }
        {
            let mut inner = self.inner.lock();
            if let Some(run) = inner.runs.get_mut(&sid) {
                if run.stdin.send(text.clone()).is_ok() {
                    run.busy = true;
                    self.refresh(&mut inner, &sid);
                    return Ok(json!({ "sid": sid, "how": "injected" }));
                }
            }
            let running_live = inner.hooks.get(&sid).map(|h| h.live && h.status == Status::Running).unwrap_or(false);
            if running_live {
                Self::hook_state(&mut inner, &sid).queue.push_back(text);
                return Ok(json!({ "sid": sid, "how": "queued" }));
            }
        }
        let cwd = self.inner.lock().sessions.get(&sid).map(|s| s.cwd.clone()).unwrap_or_default();
        if cwd.is_empty() || !Path::new(&cwd).is_dir() {
            return Err("A pasta dessa sessão não existe mais.".into());
        }
        runner::start(self, Some(sid.clone()), cwd, text, mode, None, false).await?;
        Ok(json!({ "sid": sid, "how": "resumed" }))
    }

    pub fn config_view(&self) -> Value {
        let cfg = self.cfg.read().clone();
        let link = cfg.pair_link();
        let qr = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::M)
            .map(|c| {
                c.render::<qrcode::render::svg::Color>()
                    .min_dimensions(220, 220)
                    .quiet_zone(true)
                    .dark_color(qrcode::render::svg::Color("#000000"))
                    .light_color(qrcode::render::svg::Color("#ffffff"))
                    .build()
            })
            .unwrap_or_default();
        let status = install::status(self);
        json!({
            "pcName": cfg.pc_name,
            "relayUrl": cfg.relay_url,
            "relayToken": cfg.relay_token,
            "autoAwayMin": cfg.auto_away_min,
            "autostart": cfg.autostart,
            "notifyDone": cfg.notify_done,
            "defaultMode": cfg.default_mode,
            "claudePath": cfg.claude_path,
            "pairLink": link,
            "pairQr": qr,
            "relayOnline": self.relay_online.load(Ordering::Relaxed),
            "integration": status,
            "autoUpdate": cfg.auto_update,
            "keepAwake": cfg.keep_awake,
            "dataDir": self.paths.dir.to_string_lossy(),
            "version": crate::update::VERSION,
            "update": self.update.lock().clone(),
        })
    }

    fn config_set(&self, patch: Value) -> Result<(), String> {
        let Value::Object(p) = patch else { return Err("config inválida".into()) };
        let mut relay_changed = false;
        {
            let mut cfg = self.cfg.write();
            let str_of = |k: &str| p.get(k).and_then(Value::as_str).map(|s| s.trim().to_string());
            if let Some(v) = str_of("pcName").filter(|v| !v.is_empty()) {
                cfg.pc_name = v;
            }
            if let Some(v) = str_of("relayUrl") {
                relay_changed |= v != cfg.relay_url;
                cfg.relay_url = v;
            }
            if let Some(v) = str_of("relayToken") {
                relay_changed |= v != cfg.relay_token;
                cfg.relay_token = v;
            }
            if let Some(v) = str_of("defaultMode").filter(|v| !v.is_empty()) {
                cfg.default_mode = v;
            }
            if let Some(v) = str_of("claudePath") {
                cfg.claude_path = v;
            }
            if let Some(v) = p.get("autoAwayMin").and_then(Value::as_u64) {
                cfg.auto_away_min = v.min(24 * 60) as u32;
            }
            if let Some(v) = p.get("autostart").and_then(Value::as_bool) {
                cfg.autostart = v;
            }
            if let Some(v) = p.get("notifyDone").and_then(Value::as_bool) {
                cfg.notify_done = v;
            }
            if let Some(v) = p.get("autoUpdate").and_then(Value::as_bool) {
                cfg.auto_update = v;
            }
            if let Some(v) = p.get("keepAwake").and_then(Value::as_bool) {
                cfg.keep_awake = v;
            }
            cfg.save(&self.paths);
        }
        if relay_changed {
            self.relay_wake.notify_one();
        }
        Ok(())
    }

    pub fn claude_path(&self) -> String {
        let configured = self.cfg.read().claude_path.clone();
        if !configured.trim().is_empty() {
            return configured;
        }
        if let Some(home) = dirs::home_dir() {
            let p = home.join(".local").join("bin").join("claude.exe");
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
        "claude".into()
    }

    pub fn register_run(&self, sid: &str, run: Run, cwd: &str) {
        let mut inner = self.inner.lock();
        self.ensure_session(&mut inner, sid, cwd);
        inner.runs.insert(sid.to_string(), run);
        self.refresh(&mut inner, sid);
        let msg = ev("session", json!({ "s": inner.sessions.get(sid) }));
        self.emit_all(&inner, msg);
    }

    pub fn run_result(&self, sid: &str, ok: bool, text: &str) {
        let schedule = {
            let mut inner = self.inner.lock();
            let sched = inner.runs.get_mut(sid).map(|r| {
                r.busy = false;
                r.schedule.clone()
            });
            self.refresh(&mut inner, sid);
            sched.flatten()
        };
        let title = self.session_title(sid);
        if let Some(id) = schedule {
            scheduler::mark(self, &id, if ok { "ok" } else { "erro" }, Some(sid));
            let (name, quiet) = self
                .inner
                .lock()
                .schedules
                .iter()
                .find(|s| s.id == id)
                .map(|s| (s.name.clone(), s.notify == "claude"))
                .unwrap_or((title.clone(), false));
            if ok && quiet {
                return;
            }
            self.note(
                if ok { "schedule" } else { "error" },
                format!("Agendamento {}: {name}", if ok { "concluído" } else { "falhou" }),
                one_line(text, 300),
                Some(sid.to_string()),
                None,
            );
            return;
        }
        if !ok {
            self.note("error", format!("{title}: erro"), one_line(text, 300), Some(sid.to_string()), None);
        } else if self.cfg.read().notify_done {
            self.note("done", format!("{title} terminou"), one_line(text, 300), Some(sid.to_string()), None);
        }
    }

    pub fn run_ended(&self, sid: &str, error: Option<String>) {
        let had = {
            let mut inner = self.inner.lock();
            let r = inner.runs.remove(sid);
            self.refresh(&mut inner, sid);
            r
        };
        if let (Some(run), Some(err)) = (had, error) {
            if run.busy {
                if let Some(id) = &run.schedule {
                    scheduler::mark(self, id, "erro", Some(sid));
                }
                let title = self.session_title(sid);
                self.note("error", format!("{title}: o Claude parou"), one_line(&err, 400), Some(sid.to_string()), None);
            }
        }
    }

    pub fn emit_schedule(&self, s: &Schedule) {
        let inner = self.inner.lock();
        let msg = ev("schedule", json!({ "s": s }));
        self.emit_all(&inner, msg);
    }

    pub fn emit_schedule_gone(&self, id: &str) {
        let inner = self.inner.lock();
        let msg = ev("schedule.gone", json!({ "id": id }));
        self.emit_all(&inner, msg);
    }

    pub async fn tool(self: &Arc<Self>, name: &str, args: Value) -> Result<String, String> {
        let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
        let cwd = s("cwd").unwrap_or_default();
        let sid = s("_sid").filter(|x| !x.is_empty()).or_else(|| self.guess_sid(&cwd));
        let schedule = s("_schedule").filter(|x| !x.is_empty()).or_else(|| {
            sid.as_ref().and_then(|x| self.inner.lock().runs.get(x).and_then(|r| r.schedule.clone()))
        });
        match name {
            "publish_dashboard" => self.publish_dashboard(sid, schedule, &args),
            "notify_user" => {
                let msg = s("message").ok_or("message é obrigatório")?;
                let title = s("title").unwrap_or_else(|| sid.as_deref().map(|x| self.session_title(x)).unwrap_or_else(|| "Claude".into()));
                self.note("info", title, msg, sid, None);
                Ok("Notificação enviada.".into())
            }
            "ask_user" => {
                let question = s("question").ok_or("question é obrigatório")?;
                let options: Vec<Value> = args
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|o| o.as_str()).map(|o| json!({ "label": o, "description": "" })).collect())
                    .unwrap_or_default();
                let sid = sid.unwrap_or_default();
                let title = if sid.is_empty() { "Claude".to_string() } else { self.session_title(&sid) };
                let p = Pending {
                    pid: short_id(),
                    sid: sid.clone(),
                    kind: PendingKind::Question,
                    ts: now_ms(),
                    title: title.clone(),
                    cwd,
                    tool: Some("ask_user".into()),
                    input: None,
                    questions: Some(json!([{ "question": question, "header": "Pergunta", "multiSelect": false, "options": options }])),
                    message: None,
                };
                match self.wait_pending(p, format!("{title}: pergunta"), question.clone()).await {
                    Resolution::Answers(a) => Ok(a.get(&question).and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| Value::Object(a).to_string())),
                    Resolution::Text(t) => Ok(t),
                    Resolution::Allow(_) => Ok("sim".into()),
                    Resolution::Deny(_) => Ok("não".into()),
                    Resolution::Release => Err("O usuário não respondeu.".into()),
                }
            }
            "schedule_task" => {
                let at = match s("at") {
                    Some(t) => Some(
                        chrono::DateTime::parse_from_rfc3339(&t)
                            .map(|d| d.timestamp_millis())
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M:%S")
                                    .or_else(|_| chrono::NaiveDateTime::parse_from_str(&t, "%Y-%m-%d %H:%M"))
                                    .map(|n| n.and_local_timezone(chrono::Local).single().map(|d| d.timestamp_millis()).unwrap_or(0))
                            })
                            .map_err(|_| "at deve ser ISO 8601, ex.: 2026-11-21T09:00:00")?,
                    ),
                    None => None,
                };
                let sched = Schedule {
                    name: s("name").unwrap_or_else(|| "Agendamento".into()),
                    kind: s("kind").unwrap_or_else(|| "claude".into()),
                    prompt: s("prompt").ok_or("prompt é obrigatório")?,
                    cwd: if cwd.is_empty() { s("dir").unwrap_or_default() } else { cwd },
                    at,
                    cron: s("cron"),
                    enabled: true,
                    mode: s("mode").unwrap_or_else(default_mode),
                    notify: s("notify").filter(|n| n == "always").unwrap_or_else(|| "claude".into()),
                    chrome: args.get("chrome").and_then(Value::as_bool).unwrap_or(false),
                    every: args.get("every_minutes").and_then(Value::as_u64).map(|m| m as u32),
                    days: parse_days(args.get("days")),
                    windows: args
                        .get("windows")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .filter_map(|w| w.split_once('-'))
                                .map(|(f, t)| Window { from: f.trim().to_string(), to: t.trim().to_string() })
                                .collect()
                        })
                        .unwrap_or_default(),
                    ..Default::default()
                };
                let saved = scheduler::save(self, sched)?;
                let when = saved
                    .next_run
                    .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
                    .map(|d| d.with_timezone(&chrono::Local).format("%d/%m/%Y %H:%M").to_string())
                    .unwrap_or_else(|| "sem próxima execução".into());
                Ok(format!("Agendado \"{}\" (id {}). Próxima execução: {when}.", saved.name, saved.id))
            }
            "open_browser" => tokio::task::spawn_blocking(crate::browser::ensure_browser_running)
                .await
                .map_err(|e| e.to_string())?,
            "list_schedules" => {
                let inner = self.inner.lock();
                Ok(serde_json::to_string_pretty(&inner.schedules).unwrap_or_default())
            }
            _ => Err(format!("ferramenta desconhecida: {name}")),
        }
    }

    fn guess_sid(&self, cwd: &str) -> Option<String> {
        if cwd.is_empty() {
            return None;
        }
        let inner = self.inner.lock();
        inner
            .sessions
            .values()
            .filter(|s| s.cwd.eq_ignore_ascii_case(cwd))
            .max_by_key(|s| (s.status == Status::Running, s.updated))
            .map(|s| s.sid.clone())
    }
}

fn parse_days(v: Option<&Value>) -> Vec<u8> {
    const NAMES: [&str; 7] = ["dom", "seg", "ter", "qua", "qui", "sex", "sab"];
    const EN: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    let Some(Value::Array(a)) = v else { return Vec::new() };
    a.iter()
        .filter_map(|d| match d {
            Value::Number(n) => n.as_u64().map(|n| (n % 7) as u8),
            Value::String(s) => {
                let s = s.trim().to_lowercase();
                let s = s.replace('á', "a");
                NAMES.iter().position(|n| s.starts_with(n)).or_else(|| EN.iter().position(|n| s.starts_with(n))).map(|i| i as u8)
            }
            _ => None,
        })
        .collect()
}

pub fn describe_tool(tool: &str, input: &Value) -> String {
    let g = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let text = match tool {
        "Bash" | "PowerShell" => g("command").to_string(),
        "Edit" | "Write" | "Read" | "MultiEdit" | "NotebookEdit" => g("file_path").to_string(),
        "WebFetch" => g("url").to_string(),
        "WebSearch" => g("query").to_string(),
        _ => {
            let s = input.to_string();
            if s == "null" { String::new() } else { s }
        }
    };
    one_line(&text, 200)
}
