//! Find the assistant CLIs and read their sign-in state. Nothing here spends tokens.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::Provider;
use crate::process::quiet;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CliStatus {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub signed_in: bool,
    /// "Pro", "Max", "Plus", ... when the CLI says.
    pub plan: Option<String>,
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Locate an executable on `PATH` or in the places the official installers use.
pub fn find(provider: Provider) -> Option<PathBuf> {
    let name = provider.exe();
    let exts: &[&str] = if cfg!(windows) { &["exe", "cmd", "bat"] } else { &[""] };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(h) = home() {
        dirs.push(h.join(".local").join("bin"));
        dirs.push(h.join(".claude").join("local"));
        dirs.push(h.join(".npm-global").join("bin"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    for dir in dirs {
        for ext in exts {
            let p = if ext.is_empty() { dir.join(name) } else { dir.join(format!("{name}.{ext}")) };
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// How long a status command may take before it is treated as failed (a CLI that waits
/// for input would otherwise leave the status at "Checking…" forever).
const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

fn run(exe: &Path, args: &[&str]) -> Option<(bool, String)> {
    use std::io::Read;
    let mut child = quiet(Command::new(exe).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()))
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let read_out = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out.read_to_end(&mut v);
        v
    });
    let read_err = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < RUN_TIMEOUT => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut text = String::from_utf8_lossy(&read_out.join().unwrap_or_default()).into_owned();
    text.push_str(&String::from_utf8_lossy(&read_err.join().unwrap_or_default()));
    Some((status.success(), text))
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Installed? Which version? Signed in, and to which plan?
pub fn status(provider: Provider) -> CliStatus {
    let Some(path) = find(provider) else { return CliStatus::default() };
    let version = run(&path, &["--version"]).and_then(|(ok, t)| {
        ok.then(|| t.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit())).map(str::to_string))
            .flatten()
    });
    let (signed_in, plan) = match provider {
        Provider::Claude => match run(&path, &["auth", "status"]) {
            Some((_, text)) => {
                let json = text.find('{').and_then(|i| serde_json::from_str::<Value>(&text[i..]).ok());
                match json {
                    Some(v) => (
                        v.get("loggedIn").and_then(Value::as_bool).unwrap_or(false),
                        v.get("planTier").or_else(|| v.get("subscriptionType")).and_then(Value::as_str).map(title_case),
                    ),
                    None => (false, None),
                }
            }
            None => (false, None),
        },
        Provider::Codex => match run(&path, &["login", "status"]) {
            Some((ok, text)) => {
                let lower = text.to_lowercase();
                let signed = ok && lower.contains("logged in") && !lower.contains("not logged in");
                let plan = signed.then(|| if lower.contains("chatgpt") { "ChatGPT".to_string() } else { "API key".to_string() });
                (signed, plan)
            }
            None => (false, None),
        },
    };
    CliStatus { path: Some(path), version, signed_in, plan }
}

/// Open a visible console that installs the CLI with its official installer.
pub fn install(provider: Provider) -> std::io::Result<()> {
    let script = match provider {
        Provider::Claude if cfg!(windows) => "irm https://claude.ai/install.ps1 | iex",
        Provider::Claude => "curl -fsSL https://claude.ai/install.sh | bash",
        Provider::Codex => "npm install -g @openai/codex",
    };
    open_console(script)
}

/// Open a visible console running the CLI's own sign-in (it opens the browser).
pub fn sign_in(provider: Provider) -> std::io::Result<()> {
    let exe = find(provider).map(|p| p.display().to_string()).unwrap_or_else(|| provider.exe().to_string());
    let args = match provider {
        Provider::Claude => "auth login",
        Provider::Codex => "login",
    };
    // Quote the path for the shell (a profile folder can contain an apostrophe).
    if cfg!(windows) {
        open_console(&format!("& '{}' {args}", exe.replace('\x27', "''")))
    } else {
        open_console(&format!("'{}' {args}", exe.replace('\x27', "'\''")))
    }
}

fn open_console(script: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-NoExit", "-Command", script])
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        let osa = format!("tell application \"Terminal\" to do script \"{}\"", script.replace('"', "\\\""));
        Command::new("osascript").args(["-e", &osa]).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("x-terminal-emulator").args(["-e", "sh", "-c", script]).spawn().map(|_| ())
    }
}
