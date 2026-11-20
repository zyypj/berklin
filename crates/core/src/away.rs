use std::sync::Arc;
use std::time::Duration;

use crate::agent::Agent;

#[cfg(windows)]
pub fn idle_ms() -> u64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if GetLastInputInfo(&mut info) == 0 {
            return 0;
        }
        GetTickCount().wrapping_sub(info.dwTime) as u64
    }
}

#[cfg(not(windows))]
pub fn idle_ms() -> u64 {
    0
}

pub async fn run(agent: Arc<Agent>) {
    let idle_tracking = std::env::var_os("BERKLIN_IGNORE_IDLE").is_none();
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let mut status_tick = 0u32;
    loop {
        tick.tick().await;
        status_tick += 1;
        if status_tick % 8 == 0 {
            agent.tick_status();
        }
        if !idle_tracking {
            continue;
        }
        let idle = idle_ms();
        let minutes = agent.cfg.read().auto_away_min as u64;
        let away = agent.away();
        if !away.on {
            if minutes > 0 && idle >= minutes * 60_000 {
                agent.set_away(true, true);
            }
        } else if idle < 4_000 && agent.away_since().elapsed() > Duration::from_secs(60) {
            agent.set_away(false, false);
        }
    }
}
