use std::path::PathBuf;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 47821;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub pc_name: String,
    pub relay_url: String,
    pub relay_token: String,
    pub key: String,
    pub local_port: u16,
    pub local_token: String,
    pub auto_away_min: u32,
    pub autostart: bool,
    pub notify_done: bool,
    pub claude_path: String,
    pub default_mode: String,
    pub auto_update: bool,
    pub last_version: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            pc_name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into()),
            relay_url: String::new(),
            relay_token: String::new(),
            key: random_b64(32),
            local_port: DEFAULT_PORT,
            local_token: random_b64(24),
            auto_away_min: 5,
            autostart: true,
            notify_done: true,
            claude_path: String::new(),
            default_mode: "default".into(),
            auto_update: true,
            last_version: String::new(),
        }
    }
}

pub fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

#[derive(Clone)]
pub struct Paths {
    pub dir: PathBuf,
}

impl Paths {
    pub fn new() -> Self {
        let dir = std::env::var_os("BERKLIN_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("Berklin"));
        let _ = std::fs::create_dir_all(&dir);
        Paths { dir }
    }
    pub fn config(&self) -> PathBuf {
        self.dir.join("config.json")
    }
    pub fn schedules(&self) -> PathBuf {
        self.dir.join("schedules.json")
    }
    pub fn index(&self) -> PathBuf {
        self.dir.join("index.json")
    }
    pub fn port(&self) -> PathBuf {
        self.dir.join("port")
    }
    pub fn log(&self) -> PathBuf {
        self.dir.join("berklin.log")
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Config {
        let cfg: Config = std::fs::read(paths.config())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        cfg.save(paths);
        cfg
    }

    pub fn save(&self, paths: &Paths) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let tmp = paths.config().with_extension("tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(tmp, paths.config());
            }
        }
    }

    pub fn key_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        if let Ok(v) = URL_SAFE_NO_PAD.decode(self.key.trim()) {
            if v.len() == 32 {
                out.copy_from_slice(&v);
            }
        }
        out
    }

    pub fn pair_link(&self) -> String {
        format!(
            "berklin://pair?u={}&t={}&k={}&n={}",
            urlencoding::encode(self.relay_url.trim()),
            urlencoding::encode(self.relay_token.trim()),
            self.key,
            urlencoding::encode(&self.pc_name)
        )
    }
}
