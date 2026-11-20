pub mod agent;
pub mod away;
pub mod config;
pub mod install;
pub mod model;
pub mod power;
pub mod relay;
pub mod runner;
pub mod scheduler;
pub mod server;
pub mod transcript;
pub mod update;

use std::path::PathBuf;
use std::sync::Arc;

pub use agent::Agent;
pub use model::Alert;

pub async fn start(hook_exe: PathBuf) -> Result<Arc<Agent>, String> {
    let paths = config::Paths::new();
    let cfg = config::Config::load(&paths);
    let port = cfg.local_port;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("A porta {port} já está em uso (o Berklin já está aberto?): {e}"))?;
    let _ = std::fs::write(paths.port(), port.to_string());

    let (tx_cmd, rx_cmd) = std::sync::mpsc::channel();
    let (tx_out, mut rx_out) = tokio::sync::mpsc::unbounded_channel();
    let (relay_tx, relay_rx) = tokio::sync::mpsc::unbounded_channel();
    let agent = Agent::new(paths.clone(), cfg, tx_cmd.clone(), relay_tx, hook_exe);

    transcript::Transcripts::spawn(paths.index(), tx_out, tx_cmd, rx_cmd);
    let a = agent.clone();
    tokio::spawn(async move {
        while let Some(out) = rx_out.recv().await {
            a.on_tx(out);
        }
    });
    tokio::spawn(server::serve(agent.clone(), listener));
    tokio::spawn(relay::run(agent.clone(), relay_rx));
    tokio::spawn(scheduler::run(agent.clone()));
    tokio::spawn(away::run(agent.clone()));
    tokio::spawn(update::run(agent.clone()));
    power::spawn_keep_awake(agent.clone());

    let auto_install = std::env::var_os("BERKLIN_NO_AUTOINSTALL").is_none();
    if auto_install && !install::hooks_installed(&agent) && agent.hook_exe.is_file() {
        let a = agent.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = install::install_all(&a) {
                a.log(&format!("instalação automática falhou: {e}"));
            }
        });
    }
    agent.log("agente iniciado");
    Ok(agent)
}
