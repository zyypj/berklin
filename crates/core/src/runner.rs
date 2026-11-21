use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::agent::{Agent, Run};

const IDLE_CLOSE: Duration = Duration::from_secs(20 * 60);
const UNATTENDED_PROMPT: &str = "Você está rodando em segundo plano pelo Berklin; o usuário não está olhando esta conversa. \
Quando encontrar algo que ele precisa saber (por exemplo um PR com mudanças pedidas, CI quebrado, um erro, uma tarefa concluída com resultado importante), \
avise com a ferramenta notify_user do MCP berklin: título curto e uma mensagem objetiva, no idioma dele. \
Quando precisar de uma decisão dele para continuar, use ask_user. Não notifique quando não houver nada novo. \
O usuário pode responder às suas notificações; a resposta chega como uma nova mensagem nesta conversa.";

pub async fn start(
    agent: &Arc<Agent>,
    resume: Option<String>,
    cwd: String,
    text: String,
    mode: String,
    schedule: Option<String>,
) -> Result<String, String> {
    let sid = resume.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if agent.inner.lock().runs.contains_key(&sid) {
        return Err("Essa sessão já está rodando.".into());
    }
    let mut cmd = Command::new(agent.claude_path());
    for key in inherited_session_env() {
        cmd.env_remove(key);
    }
    cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--permission-mode", &mode]);
    cmd.args(["--append-system-prompt", UNATTENDED_PROMPT]);
    if resume.is_some() {
        cmd.args(["--resume", &sid]);
    } else {
        cmd.args(["--session-id", &sid]);
    }
    cmd.current_dir(&cwd)
        .env("BERKLIN_HEADLESS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut child = cmd.spawn().map_err(|e| format!("Não consegui iniciar o claude: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("stdin")?;
    let stdout = child.stdout.take().ok_or("stdout")?;
    let stderr = child.stderr.take().ok_or("stderr")?;

    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<String>();
    let (kill_tx, mut kill_rx) = oneshot::channel::<()>();
    let _ = in_tx.send(text);
    agent.register_run(&sid, Run { stdin: in_tx, kill: Some(kill_tx), schedule, busy: true }, &cwd);

    tokio::spawn(async move {
        while let Some(t) = in_rx.recv().await {
            let line = json!({ "type": "user", "message": { "role": "user", "content": t }, "parent_tool_use_id": null }).to_string();
            if stdin.write_all(line.as_bytes()).await.is_err() || stdin.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = stdin.flush().await;
        }
    });

    let err_tail: Arc<parking_lot::Mutex<VecDeque<String>>> = Arc::new(parking_lot::Mutex::new(VecDeque::new()));
    let tail = err_tail.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let mut t = tail.lock();
            t.push_back(l);
            if t.len() > 30 {
                t.pop_front();
            }
        }
    });

    let agent = agent.clone();
    let run_sid = sid.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut killed = false;
        loop {
            tokio::select! {
                line = tokio::time::timeout(IDLE_CLOSE, lines.next_line()) => {
                    match line {
                        Ok(Ok(Some(l))) => handle_line(&agent, &run_sid, &l),
                        Ok(_) => break,
                        Err(_) => {
                            let busy = agent.inner.lock().runs.get(&run_sid).map(|r| r.busy).unwrap_or(false);
                            if !busy {
                                break;
                            }
                        }
                    }
                }
                _ = &mut kill_rx => {
                    killed = true;
                    break;
                }
            }
        }
        if killed {
            let _ = child.kill().await;
        } else if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
        let status = child.wait().await.ok();
        let failed = !killed && status.map(|s| !s.success()).unwrap_or(true);
        let error = if killed {
            Some("Interrompido pelo usuário.".to_string())
        } else if failed {
            let t = err_tail.lock();
            Some(if t.is_empty() { "O processo terminou com erro.".into() } else { t.iter().cloned().collect::<Vec<_>>().join("\n") })
        } else {
            None
        };
        agent.run_ended(&run_sid, error);
    });

    Ok(sid)
}

pub fn inherited_session_env() -> Vec<String> {
    if std::env::var_os("CLAUDECODE").is_none() {
        return Vec::new();
    }
    std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| k.starts_with("CLAUDE") && k != "CLAUDE_CONFIG_DIR")
        .collect()
}

fn handle_line(agent: &Arc<Agent>, sid: &str, line: &str) {
    if !line.starts_with("{\"type\":\"result\"") && !line.contains("\"type\":\"result\"") {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else { return };
    if v.get("type").and_then(Value::as_str) != Some("result") {
        return;
    }
    let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
        || v.get("subtype").and_then(Value::as_str).map(|s| s != "success").unwrap_or(false);
    let text = v
        .get("result")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| v.get("subtype").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    agent.run_result(sid, !is_error, &text);
}
