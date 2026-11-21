use std::path::PathBuf;
use std::process::Command;

const HOST: &str = "com.anthropic.claude_code_browser_extension";
const CHROME_KEY: &str = "HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts";

struct Browser {
    key: &'static str,
    user_data: &'static [&'static str],
}

const BROWSERS: &[Browser] = &[
    Browser { key: "HKCU\\Software\\BraveSoftware\\Brave-Browser\\NativeMessagingHosts", user_data: &["BraveSoftware", "Brave-Browser"] },
    Browser { key: "HKCU\\Software\\Microsoft\\Edge\\NativeMessagingHosts", user_data: &["Microsoft", "Edge"] },
];

fn reg(args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("reg");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn default_value(key: &str) -> Option<String> {
    let text = reg(&["query", key, "/ve"])?;
    text.lines()
        .find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

fn installed(parts: &[&str]) -> bool {
    let mut p: PathBuf = dirs::data_local_dir().unwrap_or_default();
    for part in parts {
        p.push(part);
    }
    p.push("User Data");
    p.is_dir()
}

pub fn mirror_claude_native_host() -> Vec<&'static str> {
    let Some(manifest) = default_value(&format!("{CHROME_KEY}\\{HOST}")) else { return Vec::new() };
    let mut done = Vec::new();
    for b in BROWSERS {
        if !installed(b.user_data) {
            continue;
        }
        let key = format!("{}\\{HOST}", b.key);
        if default_value(&key).as_deref() == Some(manifest.as_str()) {
            continue;
        }
        if reg(&["add", &key, "/ve", "/t", "REG_SZ", "/d", &manifest, "/f"]).is_some() {
            done.push(b.key);
        }
    }
    done
}
