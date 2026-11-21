use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Waiting,
    #[default]
    Idle,
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct Session {
    pub sid: String,
    pub title: String,
    pub cwd: String,
    pub project: String,
    pub updated: i64,
    pub status: Status,
    pub live: bool,
    pub headless: bool,
    pub preview: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tokens: Option<u64>,
}

#[derive(Clone, Serialize, Debug)]
pub struct Item {
    pub i: u32,
    pub k: &'static str,
    pub ts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub err: Option<bool>,
}

impl Item {
    pub fn new(k: &'static str, ts: i64) -> Self {
        Item { i: 0, k, ts, text: None, name: None, input: None, tid: None, err: None }
    }
}

#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum PendingKind {
    Permission,
    Question,
    Stop,
}

#[derive(Clone, Serialize, Debug)]
pub struct Pending {
    pub pid: String,
    pub sid: String,
    pub kind: PendingKind,
    pub ts: i64,
    pub title: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub questions: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug)]
pub enum Resolution {
    Allow(Option<String>),
    Deny(Option<String>),
    Answers(Map<String, Value>),
    Text(String),
    Release,
}

#[derive(Clone, Serialize, Debug)]
pub struct Note {
    pub nid: String,
    pub ts: i64,
    pub kind: &'static str,
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<String>,
}

#[derive(Clone, Copy, Serialize, Default, Debug)]
pub struct Away {
    pub on: bool,
    pub auto: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_schedule_kind")]
    pub kind: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_notify")]
    pub notify: String,
    #[serde(default)]
    pub chrome: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sid: Option<String>,
}

fn default_schedule_kind() -> String {
    "claude".into()
}

fn default_true() -> bool {
    true
}

pub fn default_notify() -> String {
    "always".into()
}

pub fn default_mode() -> String {
    "default".into()
}

#[derive(Clone)]
pub struct Alert {
    pub title: String,
    pub body: String,
    pub sid: Option<String>,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + 24);
    out.push_str(&s[..end]);
    out.push_str("\n… (cortado)");
    out
}

pub fn one_line(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(max.min(s.len()));
    let mut last_space = false;
    for ch in s.chars() {
        if out.len() >= max {
            out.push('…');
            break;
        }
        let ch = if ch.is_whitespace() { ' ' } else { ch };
        if ch == ' ' && (last_space || out.is_empty()) {
            continue;
        }
        last_space = ch == ' ';
        out.push(ch);
    }
    out.trim_end().to_string()
}
