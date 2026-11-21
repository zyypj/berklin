use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MAX_BYTES: usize = 400 * 1024;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub key: String,
    pub name: String,
    pub title: String,
    pub ts: i64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sched: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    pub content: Value,
}

impl Dashboard {
    pub fn meta(&self) -> Value {
        json!({
            "key": self.key,
            "name": self.name,
            "title": self.title,
            "ts": self.ts,
            "kind": self.kind,
            "sched": self.sched,
            "sid": self.sid,
        })
    }
}

fn dir(base: &Path) -> PathBuf {
    base.join("dashboards")
}

fn file_for(base: &Path, key: &str) -> PathBuf {
    let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    dir(base).join(format!("{safe}.json"))
}

pub fn load_all(base: &Path) -> HashMap<String, Dashboard> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir(base)) else { return out };
    for e in entries.flatten() {
        if let Some(d) = std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Dashboard>(&b).ok()) {
            out.insert(d.key.clone(), d);
        }
    }
    out
}

pub fn save(base: &Path, d: &Dashboard) -> Result<(), String> {
    let _ = std::fs::create_dir_all(dir(base));
    let bytes = serde_json::to_vec(d).map_err(|e| e.to_string())?;
    let path = file_for(base, &d.key);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn remove(base: &Path, key: &str) {
    let _ = std::fs::remove_file(file_for(base, key));
}

pub fn content_from_args(args: &Value) -> Result<(String, Value, String), String> {
    let html = args.get("html").and_then(Value::as_str).filter(|h| !h.trim().is_empty());
    let data = args.get("data").filter(|d| !d.is_null());
    let (kind, content) = match (html, data) {
        (Some(h), _) => ("html", Value::String(h.to_string())),
        (None, Some(Value::String(s))) => (
            "json",
            serde_json::from_str::<Value>(s).map_err(|e| format!("data não é um JSON válido: {e}"))?,
        ),
        (None, Some(d)) => ("json", d.clone()),
        (None, None) => return Err("Envie `data` (JSON do painel) ou `html` (HTML + CSS).".into()),
    };
    if content.to_string().len() > MAX_BYTES {
        return Err("O painel passou de 400 KB; resuma o conteúdo.".into());
    }
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| content.get("title").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    Ok((kind.to_string(), content, title))
}
