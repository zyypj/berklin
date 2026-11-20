use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::agent::Agent;
use crate::model::now_ms;

pub const REPO: &str = "zyypj/berklin";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const ASSET_SUFFIX: &str = "windows-x64.zip";
const FILES: [&str; 2] = ["berklin-hook.exe", "berklin.exe"];
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
const RETRY_BUSY: Duration = Duration::from_secs(5 * 60);
const MAX_DOWNLOAD: u64 = 150 * 1024 * 1024;

#[derive(Clone, Serialize, Default, Debug)]
pub struct UpdateState {
    pub current: String,
    pub latest: Option<String>,
    pub available: bool,
    pub ready: bool,
    pub checked: Option<i64>,
    pub error: Option<String>,
    #[serde(skip)]
    pub url: Option<String>,
    #[serde(skip)]
    pub staged: Option<PathBuf>,
}

fn parse_version(v: &str) -> (u64, u64, u64) {
    let mut it = v.trim().trim_start_matches('v').split(|c: char| c == '.' || c == '-').map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

fn fetch_latest() -> Result<(String, Option<String>), String> {
    let resp = ureq::get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("User-Agent", "Berklin-Windows")
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .call()
        .map_err(|e| format!("GitHub: {e}"))?;
    let v: Value = resp.into_json().map_err(|e| e.to_string())?;
    let tag = v.get("tag_name").and_then(Value::as_str).ok_or("release sem tag")?.trim_start_matches('v').to_string();
    let url = v
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|x| x.get("name").and_then(Value::as_str).map(|n| n.ends_with(ASSET_SUFFIX)).unwrap_or(false)))
        .and_then(|x| x.get("browser_download_url"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((tag, url))
}

fn stage(dir: &Path, version: &str, url: &str) -> Result<PathBuf, String> {
    let staging = dir.join("update").join(version);
    if FILES.iter().all(|f| staging.join(f).is_file()) {
        return Ok(staging);
    }
    let _ = std::fs::remove_dir_all(dir.join("update"));
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let resp = ureq::get(url)
        .set("User-Agent", "Berklin-Windows")
        .timeout(Duration::from_secs(300))
        .call()
        .map_err(|e| format!("download: {e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader().take(MAX_DOWNLOAD).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("zip: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = entry.enclosed_name().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())) else { continue };
        if !FILES.contains(&name.as_str()) {
            continue;
        }
        let mut out = std::fs::File::create(staging.join(&name)).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }
    if !FILES.iter().all(|f| staging.join(f).is_file()) {
        return Err("o pacote da atualização está incompleto".into());
    }
    Ok(staging)
}

fn install_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

pub fn is_dev_build() -> bool {
    std::env::current_exe()
        .map(|p| p.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target")))
        .unwrap_or(true)
}

pub fn cleanup_old() {
    let Some(dir) = install_dir() else { return };
    for f in FILES {
        let _ = std::fs::remove_file(dir.join(format!("{f}.old")));
    }
}

fn swap_files(staging: &Path) -> Result<PathBuf, String> {
    let dir = install_dir().ok_or("pasta do app desconhecida")?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for f in FILES {
        let target = dir.join(f);
        let old = dir.join(format!("{f}.old"));
        let _ = std::fs::remove_file(&old);
        if target.exists() {
            if let Err(e) = std::fs::rename(&target, &old) {
                for (t, o) in moved.iter().rev() {
                    let _ = std::fs::rename(o, t);
                }
                return Err(format!("não consegui trocar {f}: {e}"));
            }
            moved.push((target.clone(), old));
        }
        if let Err(e) = std::fs::copy(staging.join(f), &target) {
            for (t, o) in moved.iter().rev() {
                let _ = std::fs::remove_file(t);
                let _ = std::fs::rename(o, t);
            }
            return Err(format!("não consegui copiar {f}: {e}"));
        }
    }
    Ok(dir.join("berklin.exe"))
}

fn relaunch(exe: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--hidden").arg("--wait-pid").arg(std::process::id().to_string());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(windows)]
pub fn wait_for_pid(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return;
        }
        WaitForSingleObject(h, 20_000);
        CloseHandle(h);
    }
}

#[cfg(not(windows))]
pub fn wait_for_pid(_pid: u32) {}

pub async fn check(agent: &Arc<Agent>) -> UpdateState {
    let dir = agent.paths.dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let (latest, url) = fetch_latest()?;
        let staged = match (&url, is_newer(&latest, VERSION)) {
            (Some(u), true) => Some(stage(&dir, &latest, u)?),
            _ => None,
        };
        Ok::<_, String>((latest, url, staged))
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    let mut st = agent.update.lock();
    st.current = VERSION.to_string();
    st.checked = Some(now_ms());
    match result {
        Ok((latest, url, staged)) => {
            st.available = is_newer(&latest, VERSION);
            st.latest = Some(latest);
            st.url = url;
            st.ready = staged.is_some();
            st.staged = staged;
            st.error = None;
        }
        Err(e) => st.error = Some(e),
    }
    st.clone()
}

pub fn apply(agent: &Arc<Agent>) -> Result<(), String> {
    let (staged, latest) = {
        let st = agent.update.lock();
        (st.staged.clone().ok_or("Nenhuma atualização baixada.")?, st.latest.clone().unwrap_or_default())
    };
    if is_dev_build() {
        return Err("Build de desenvolvimento: instale pelo scripts/install.ps1 para atualizar sozinho.".into());
    }
    let exe = swap_files(&staged)?;
    agent.log(&format!("atualizando para {latest}"));
    relaunch(&exe)?;
    agent.restart.notify_one();
    Ok(())
}

fn idle(agent: &Agent) -> bool {
    let inner = agent.inner.lock();
    inner.runs.is_empty() && agent.pending_count(&inner) == 0
}

pub async fn run(agent: Arc<Agent>) {
    cleanup_old();
    let previous = agent.cfg.read().last_version.clone();
    if previous != VERSION {
        if !previous.is_empty() {
            agent.note("info", format!("Berklin atualizado para {VERSION}"), format!("Versão anterior: {previous}"), None, None);
        }
        let mut cfg = agent.cfg.write();
        cfg.last_version = VERSION.to_string();
        cfg.save(&agent.paths);
    }
    agent.update.lock().current = VERSION.to_string();
    tokio::time::sleep(Duration::from_secs(20)).await;
    loop {
        let st = check(&agent).await;
        let mut wait = CHECK_EVERY;
        if st.ready && agent.cfg.read().auto_update && !is_dev_build() {
            if idle(&agent) {
                match apply(&agent) {
                    Ok(()) => return,
                    Err(e) => {
                        agent.log(&format!("atualização falhou: {e}"));
                        agent.update.lock().error = Some(e);
                    }
                }
            } else {
                wait = RETRY_BUSY;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = agent.update_wake.notified() => {}
        }
    }
}
