use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::agent::Agent;

#[cfg(windows)]
pub fn in_app_package() -> bool {
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    const APPMODEL_ERROR_NO_PACKAGE: u32 = 15700;
    let mut len: u32 = 0;
    let rc = unsafe { GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) };
    rc != APPMODEL_ERROR_NO_PACKAGE
}

#[cfg(not(windows))]
pub fn in_app_package() -> bool {
    false
}

pub fn relaunch_outside_package() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    Command::new("explorer.exe").arg(exe).spawn().is_ok()
}

#[cfg(windows)]
fn set_awake(on: bool) {
    use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
    unsafe {
        SetThreadExecutionState(if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS });
    }
}

#[cfg(not(windows))]
fn set_awake(_on: bool) {}

pub fn spawn_keep_awake(agent: Arc<Agent>) {
    let _ = std::thread::Builder::new().name("berklin-awake".into()).spawn(move || {
        let mut last = None;
        loop {
            let on = agent.cfg.read().keep_awake;
            if last != Some(on) {
                set_awake(on);
                last = Some(on);
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

fn powercfg(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("powercfg");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("powercfg: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!("powercfg falhou: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

pub fn lid_actions() -> Option<(u32, u32)> {
    let text = powercfg(&["/qh", "SCHEME_CURRENT", "SUB_BUTTONS", "LIDACTION"]).ok()?;
    let values: Vec<u32> = text
        .lines()
        .filter_map(|l| l.trim().rsplit(' ').next())
        .filter_map(|tok| tok.strip_prefix("0x"))
        .filter_map(|hex| u32::from_str_radix(hex, 16).ok())
        .collect();
    if values.len() < 2 {
        return None;
    }
    Some((values[values.len() - 2], values[values.len() - 1]))
}

pub fn set_lid_stay_awake_on_ac(stay_awake: bool) -> Result<(), String> {
    let value = if stay_awake { "0" } else { "1" };
    powercfg(&["/setacvalueindex", "SCHEME_CURRENT", "SUB_BUTTONS", "LIDACTION", value])?;
    powercfg(&["/setactive", "SCHEME_CURRENT"])?;
    Ok(())
}
