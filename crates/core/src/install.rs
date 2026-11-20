use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Map, Value};

use crate::agent::Agent;

const EVENTS: &[(&str, Option<&str>, u64)] = &[
    ("SessionStart", None, 10),
    ("SessionEnd", None, 5),
    ("UserPromptSubmit", None, 10),
    ("Notification", None, 10),
    ("Stop", None, 86_400),
    ("PermissionRequest", None, 86_400),
    ("PreToolUse", Some("AskUserQuestion"), 86_400),
];

const MARK: &str = "berklin-hook";

fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
}

fn settings_path() -> PathBuf {
    claude_dir().join("settings.json")
}

fn hook_command(agent: &Agent, event: &str) -> String {
    format!("\"{}\" {}", agent.hook_exe.to_string_lossy().replace('\\', "/"), event)
}

fn read_settings() -> Result<Map<String, Value>, String> {
    match std::fs::read(settings_path()) {
        Ok(b) if !b.iter().all(u8::is_ascii_whitespace) => match serde_json::from_slice::<Value>(&b) {
            Ok(Value::Object(m)) => Ok(m),
            _ => Err("~/.claude/settings.json não é um JSON válido; corrija antes de instalar.".into()),
        },
        _ => Ok(Map::new()),
    }
}

fn write_settings(m: &Map<String, Value>) -> Result<(), String> {
    let path = settings_path();
    let backup = path.with_extension("json.berklin-backup");
    if path.is_file() && !backup.exists() {
        let _ = std::fs::copy(&path, &backup);
    }
    let mut text = serde_json::to_string_pretty(&Value::Object(m.clone())).map_err(|e| e.to_string())?;
    text.push('\n');
    let tmp = path.with_extension("json.berklin-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn strip_ours(hooks: &mut Map<String, Value>) {
    for (_, groups) in hooks.iter_mut() {
        let Value::Array(list) = groups else { continue };
        for g in list.iter_mut() {
            if let Some(Value::Array(inner)) = g.get_mut("hooks") {
                inner.retain(|h| !h.get("command").and_then(Value::as_str).map(|c| c.contains(MARK)).unwrap_or(false));
            }
        }
        list.retain(|g| g.get("hooks").and_then(Value::as_array).map(|a| !a.is_empty()).unwrap_or(true));
    }
    hooks.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));
}

pub fn install_hooks(agent: &Agent) -> Result<(), String> {
    if !agent.hook_exe.is_file() {
        return Err(format!("Não achei {}", agent.hook_exe.display()));
    }
    let mut settings = read_settings()?;
    let hooks = settings.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(hooks) = hooks else { return Err("\"hooks\" em settings.json não é um objeto.".into()) };
    strip_ours(hooks);
    for (event, matcher, timeout) in EVENTS {
        let mut group = Map::new();
        if let Some(m) = matcher {
            group.insert("matcher".into(), json!(m));
        }
        group.insert(
            "hooks".into(),
            json!([{ "type": "command", "command": hook_command(agent, event), "timeout": timeout }]),
        );
        let list = hooks.entry(event.to_string()).or_insert_with(|| Value::Array(vec![]));
        if let Value::Array(list) = list {
            list.push(Value::Object(group));
        }
    }
    write_settings(&settings)
}

pub fn remove_hooks() -> Result<(), String> {
    let mut settings = read_settings()?;
    if let Some(Value::Object(hooks)) = settings.get_mut("hooks") {
        strip_ours(hooks);
        if hooks.is_empty() {
            settings.remove("hooks");
        }
    }
    write_settings(&settings)
}

fn claude(agent: &Agent, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(agent.claude_path());
    for key in crate::runner::inherited_session_env() {
        cmd.env_remove(key);
    }
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("claude: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(text)
    } else {
        Err(text)
    }
}

pub fn install_mcp(agent: &Agent) -> Result<(), String> {
    let _ = claude(agent, &["mcp", "remove", "--scope", "user", "berklin"]);
    let exe = agent.hook_exe.to_string_lossy().into_owned();
    claude(agent, &["mcp", "add", "--scope", "user", "berklin", "--", &exe, "mcp"]).map(|_| ())
}

pub fn install_all(agent: &Agent) -> Result<(), String> {
    install_hooks(agent)?;
    install_mcp(agent).map_err(|e| format!("Hooks instalados, mas o MCP falhou: {e}"))
}

pub fn remove_all(agent: &Agent) -> Result<(), String> {
    remove_hooks()?;
    let _ = claude(agent, &["mcp", "remove", "--scope", "user", "berklin"]);
    Ok(())
}

pub fn hooks_installed(agent: &Agent) -> bool {
    let Ok(settings) = read_settings() else { return false };
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else { return false };
    EVENTS.iter().all(|(event, _, _)| {
        let want = hook_command(agent, event);
        hooks
            .get(*event)
            .and_then(Value::as_array)
            .map(|groups| {
                groups.iter().any(|g| {
                    g.get("hooks")
                        .and_then(Value::as_array)
                        .map(|hs| hs.iter().any(|h| h.get("command").and_then(Value::as_str) == Some(want.as_str())))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    })
}

pub fn mcp_installed() -> bool {
    let path = dirs::home_dir().unwrap_or_default().join(".claude.json");
    let Ok(bytes) = std::fs::read(path) else { return false };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else { return false };
    v.get("mcpServers").and_then(|m| m.get("berklin")).is_some()
}

pub fn status(agent: &Agent) -> Value {
    json!({
        "hooks": hooks_installed(agent),
        "mcp": mcp_installed(),
        "hookExe": agent.hook_exe.to_string_lossy(),
        "settings": settings_path().to_string_lossy(),
    })
}
