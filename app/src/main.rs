#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::time::Duration;

use berklin_core::Agent;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_notification::NotificationExt;

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg(windows)]
fn apply_autostart(enabled: bool) {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let Ok(run) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Software\\Microsoft\\Windows\\CurrentVersion\\Run", winreg::enums::KEY_ALL_ACCESS)
    else {
        return;
    };
    if enabled {
        if let Ok(exe) = std::env::current_exe() {
            let _ = run.set_value("Berklin", &format!("\"{}\" --hidden", exe.display()));
        }
    } else {
        let _ = run.delete_value("Berklin");
    }
}

#[cfg(not(windows))]
fn apply_autostart(_enabled: bool) {}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pid) = args.iter().position(|a| a == "--wait-pid").and_then(|i| args.get(i + 1)).and_then(|p| p.parse().ok()) {
        berklin_core::update::wait_for_pid(pid);
    }
    if berklin_core::power::in_app_package()
        && std::env::var_os("BERKLIN_ALLOW_PACKAGED").is_none()
        && berklin_core::power::relaunch_outside_package()
    {
        return;
    }
    let hidden = args.iter().any(|a| a == "--hidden");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show(app)))
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let hook_exe = std::env::current_exe()?.with_file_name("berklin-hook.exe");
            let agent: Arc<Agent> = match tauri::async_runtime::block_on(berklin_core::start(hook_exe)) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            app.manage(agent.clone());

            let cfg = agent.cfg.read().clone();
            apply_autostart(cfg.autostart);
            let init = format!(
                "window.__BERKLIN_LOCAL={{port:{},token:{:?},platform:'windows'}};",
                cfg.local_port, cfg.local_token
            );
            let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Berklin")
                .inner_size(1180.0, 800.0)
                .min_inner_size(380.0, 520.0)
                .initialization_script(&init)
                .visible(!hidden)
                .build()?;
            let w = win.clone();
            win.on_window_event(move |e| {
                if let WindowEvent::CloseRequested { api, .. } = e {
                    api.prevent_close();
                    let _ = w.hide();
                }
            });

            let open_i = MenuItem::with_id(app, "open", "Abrir Berklin", true, None::<&str>)?;
            let away_i = CheckMenuItem::with_id(app, "away", "Modo ausente", true, false, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_i, &away_i, &quit_i])?;
            let menu_agent = agent.clone();
            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().cloned().expect("icon"))
                .tooltip("Berklin")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "away" => {
                        let on = !menu_agent.away().on;
                        menu_agent.set_away(on, false);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;

            let sync_agent = agent.clone();
            let sync_item = away_i.clone();
            tauri::async_runtime::spawn(async move {
                let mut last = None;
                let mut last_autostart = cfg.autostart;
                loop {
                    let on = sync_agent.away().on;
                    if last != Some(on) {
                        let _ = sync_item.set_checked(on);
                        last = Some(on);
                    }
                    let autostart = sync_agent.cfg.read().autostart;
                    if autostart != last_autostart {
                        apply_autostart(autostart);
                        last_autostart = autostart;
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            });

            let restart_handle = app.handle().clone();
            let restart_agent = agent.clone();
            tauri::async_runtime::spawn(async move {
                restart_agent.restart.notified().await;
                restart_handle.exit(0);
            });

            let handle = app.handle().clone();
            let mut alerts = agent.alerts.subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    match alerts.recv().await {
                        Ok(a) => {
                            let focused = handle
                                .get_webview_window("main")
                                .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
                                .unwrap_or(false);
                            if !focused {
                                let _ = handle.notification().builder().title(&a.title).body(&a.body).show();
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("berklin");
}
