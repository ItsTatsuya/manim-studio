//! Crash reports, kept on this computer. A panic writes a short report (version, OS,
//! message, location, backtrace) to the data folder; nothing is sent anywhere. The
//! teacher can choose to open a pre-filled GitHub issue from Settings → About.

use std::path::PathBuf;

/// Where the project's issues live.
pub const ISSUES_URL: &str = "https://github.com/ItsTatsuya/manim-studio/issues/new";

fn dir() -> Option<PathBuf> {
    crate::store::data_dir().map(|d| d.join("crashes"))
}

/// Install the panic hook (call first thing in `main`).
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let report = format!(
            "Manim Studio {}\nOS: {} {}\nThread: {thread}\nPanic: {message}\nAt: {location}\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::backtrace::Backtrace::force_capture()
        );
        if let Some(d) = dir() {
            let _ = std::fs::create_dir_all(&d);
            let _ = std::fs::write(d.join(format!("crash-{}.txt", crate::store::now())), &report);
        }
        previous(info);
    }));
}

/// The newest crash report, if any.
pub fn latest() -> Option<(PathBuf, String)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir()?)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    files.sort();
    let p = files.pop()?;
    let text = std::fs::read_to_string(&p).ok()?;
    Some((p, text))
}

/// A GitHub "new issue" link with the version, OS and (when there is one) the first
/// lines of the latest crash report filled in. Opening it sends nothing by itself.
pub fn issue_url() -> String {
    let mut body = format!(
        "**What happened?**\n\n\n**What did you expect?**\n\n\n---\nManim Studio {} · {} {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if let Some((_, report)) = latest() {
        let head: String = report.lines().take(6).collect::<Vec<_>>().join("\n");
        body.push_str(&format!("\nLatest crash report:\n```\n{head}\n```\n"));
    }
    format!("{ISSUES_URL}?body={}", encode(&body))
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Open a URL in the default browser.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}
