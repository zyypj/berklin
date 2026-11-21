use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const HOST: &str = "com.anthropic.claude_code_browser_extension";
const EXTENSION_ID: &str = "fcoeoabgfenejglbffodgkkbkcdhcgfn";

struct Launchable {
    name: &'static str,
    process: &'static str,
    user_data: &'static [&'static str],
    exe: &'static [&'static str],
}

const LAUNCH_ORDER: &[Launchable] = &[
    Launchable {
        name: "Brave",
        process: "brave.exe",
        user_data: &["BraveSoftware", "Brave-Browser"],
        exe: &["BraveSoftware\\Brave-Browser\\Application\\brave.exe"],
    },
    Launchable {
        name: "Chrome",
        process: "chrome.exe",
        user_data: &["Google", "Chrome"],
        exe: &["Google\\Chrome\\Application\\chrome.exe"],
    },
    Launchable {
        name: "Edge",
        process: "msedge.exe",
        user_data: &["Microsoft", "Edge"],
        exe: &["Microsoft\\Edge\\Application\\msedge.exe"],
    },
];

fn user_data_dir(parts: &[&str]) -> PathBuf {
    let mut p: PathBuf = dirs::data_local_dir().unwrap_or_default();
    for part in parts {
        p.push(part);
    }
    p.push("User Data");
    p
}

fn has_claude_extension(parts: &[&str]) -> bool {
    let Ok(profiles) = std::fs::read_dir(user_data_dir(parts)) else { return false };
    profiles.flatten().any(|p| p.path().join("Extensions").join(EXTENSION_ID).is_dir())
}

fn find_exe(rel: &[&str]) -> Option<PathBuf> {
    let roots = [
        dirs::data_local_dir(),
        std::env::var_os("ProgramFiles").map(PathBuf::from),
        std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
    ];
    rel.iter()
        .flat_map(|r| roots.iter().flatten().map(move |root| root.join(r)))
        .find(|p| p.is_file())
}

fn is_running(process: &str) -> bool {
    let mut cmd = Command::new("tasklist");
    cmd.args(["/FI", &format!("IMAGENAME eq {process}"), "/NH"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains(process))
        .unwrap_or(false)
}

pub fn ensure_browser_running() -> Result<String, String> {
    let candidates: Vec<&Launchable> = LAUNCH_ORDER.iter().filter(|b| has_claude_extension(b.user_data)).collect();
    if candidates.is_empty() {
        return Err("Nenhum navegador com a extensão Claude in Chrome instalada (Brave, Chrome ou Edge).".into());
    }
    if let Some(b) = candidates.iter().find(|b| is_running(b.process)) {
        return Ok(format!("{} já está aberto.", b.name));
    }
    for b in candidates {
        let Some(exe) = find_exe(b.exe) else { continue };
        let mut cmd = Command::new(&exe);
        cmd.arg("--no-startup-window");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0000_0008 | 0x0000_0200);
        }
        if cmd.spawn().is_ok() {
            mirror_claude_native_host();
            std::thread::sleep(Duration::from_secs(8));
            return Ok(format!("{} aberto em segundo plano.", b.name));
        }
    }
    Err("Não consegui abrir o navegador com a extensão do Claude.".into())
}
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
