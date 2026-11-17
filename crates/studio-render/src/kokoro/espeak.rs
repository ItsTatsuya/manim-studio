//! espeak-ng (GPL-3.0) for words misaki's dictionaries don't know. It is only ever run as a
//! separate program. On Windows Studio can fetch the official release and unpack it into its
//! data folder (an administrative extract of the MSI: nothing is installed system-wide).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const MSI_URL: &str = "https://github.com/espeak-ng/espeak-ng/releases/download/1.52.0/espeak-ng.msi";
const MSI_SIZE: u64 = 12_765_862;
const MSI_SHA256: &str = "7f673c709ea5dd579d3b5ebb98688cc575328a6ab7438d2bc405b88cedaeafb9";

/// Download size, for the button.
pub const DOWNLOAD_SIZE: u64 = MSI_SIZE;
/// Studio can install it itself on this platform.
pub const CAN_INSTALL: bool = cfg!(windows);

#[derive(Clone, Debug, PartialEq)]
pub struct Espeak {
    pub exe: PathBuf,
    /// `--path` for a copy unpacked by Studio (its data sits next to the program).
    data: Option<PathBuf>,
    /// Unpacked by Studio rather than found on the system.
    pub bundled: bool,
}

/// Studio's own copy: `<data>/voices/espeak-ng`, beside the Kokoro folder.
pub fn local_dir(kokoro_dir: &Path) -> PathBuf {
    kokoro_dir.parent().unwrap_or(kokoro_dir).join("espeak-ng")
}

/// Studio's copy first, then one on `PATH` or in the default install folder.
pub fn find(kokoro_dir: Option<&Path>) -> Option<Espeak> {
    let exe_name = if cfg!(windows) { "espeak-ng.exe" } else { "espeak-ng" };
    if let Some(local) = kokoro_dir.map(local_dir) {
        let exe = local.join(exe_name);
        if exe.is_file() && local.join("espeak-ng-data").is_dir() {
            return Some(Espeak { exe, data: Some(local), bundled: true });
        }
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if cfg!(windows) {
        dirs.push(r"C:\Program Files\eSpeak NG".into());
    }
    dirs.into_iter().map(|d| d.join(exe_name)).find(|p| p.is_file()).map(|exe| Espeak { exe, data: None, bundled: false })
}

/// Longest one word may take. A hung espeak-ng would otherwise block narration for good.
const WORD_TIMEOUT: Duration = Duration::from_secs(10);

impl Espeak {
    /// IPA for one word, with `^` tying multi-letter phonemes. `None` if espeak-ng fails
    /// or takes longer than 10 s (it is killed).
    pub fn ipa(&self, word: &str, british: bool) -> Option<String> {
        let mut cmd = Command::new(&self.exe);
        if let Some(data) = &self.data {
            cmd.arg(format!("--path={}", data.display()));
        }
        cmd.args(["-q", "--ipa", "--tie=^", "-v", if british { "en-gb" } else { "en-us" }, word]);
        let out = run_with_timeout(&mut cmd, WORD_TIMEOUT)?;
        let s = String::from_utf8_lossy(&out).split_whitespace().collect::<Vec<_>>().join(" ");
        (!s.is_empty()).then_some(s)
    }
}

/// Run `cmd` and return its stdout if it exits successfully within `limit`; otherwise
/// kill it and return `None`.
fn run_with_timeout(cmd: &mut Command, limit: Duration) -> Option<Vec<u8>> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    crate::export::quiet(cmd);
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // Read on a thread so a full pipe can't stall the poll loop below.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::take(&mut stdout, 1 << 20).read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if started.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = reader.join().unwrap_or_default();
    status.filter(|st| st.success()).map(|_| out)
}

/// Download the official Windows release and unpack it into [`local_dir`].
/// `progress(done_bytes, total_bytes)`. The download resumes a leftover `.part`, times
/// out when stalled, and stops with [`crate::Cancelled`] once `cancel` is set.
pub fn install(kokoro_dir: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    if !CAN_INSTALL {
        bail!("install espeak-ng with your package manager");
    }
    let dir = local_dir(kokoro_dir);
    let parent = dir.parent().context("no voices folder")?;
    std::fs::create_dir_all(parent)?;
    let msi = parent.join("espeak-ng.msi");
    // msiexec only opens files named .msi, so the finished download is `espeak-ng.msi`
    // (its in-progress copy is `espeak-ng.msi.part`).
    let mut done = 0u64;
    crate::fetch::download_file(MSI_URL, &msi, MSI_SIZE, MSI_SHA256, cancel, |n| {
        done += n;
        progress(done.min(MSI_SIZE), MSI_SIZE);
    })?;
    // An administrative install only unpacks the files into TARGETDIR.
    let unpack = parent.join("espeak-ng.unpack");
    let _ = std::fs::remove_dir_all(&unpack);
    let mut cmd = Command::new("msiexec");
    cmd.arg("/a").arg(&msi).arg("/qn");
    // msiexec wants PROPERTY="value with spaces", not the whole argument quoted.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::raw_arg(&mut cmd, format!("TARGETDIR=\"{}\"", unpack.display()));
    crate::export::quiet(&mut cmd);
    let status = cmd.status().context("running msiexec")?;
    let _ = std::fs::remove_file(&msi);
    let inner = unpack.join("eSpeak NG");
    if !status.success() || !inner.join("espeak-ng.exe").is_file() {
        let _ = std::fs::remove_dir_all(&unpack);
        bail!("couldn't unpack espeak-ng (msiexec exit {:?})", status.code());
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&inner, &dir)?;
    let _ = std::fs::remove_dir_all(&unpack);
    // Smoke test: a crash here means the data folder is wrong.
    let found = find(Some(kokoro_dir)).context("espeak-ng is missing after unpacking")?;
    found.ipa("hello", false).context("espeak-ng didn't run")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command that outlives the limit is killed and reported as no result.
    #[test]
    fn hung_processes_are_killed() {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let t = Instant::now();
        assert!(run_with_timeout(&mut cmd, Duration::from_millis(300)).is_none());
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    /// `ESPEAK_INSTALL=<kokoro dir>`: unpack Studio's copy there, then read an unknown word.
    #[test]
    fn installs_and_reads_unknown_words() {
        let Some(dir) = std::env::var_os("ESPEAK_INSTALL") else { return };
        let dir = PathBuf::from(dir);
        let mut calls = 0;
        install(&dir, &AtomicBool::new(false), |_, _| calls += 1).unwrap();
        assert!(calls > 0 && find(Some(&dir)).is_some_and(|e| e.bundled));
        if dir.join("us_gold.json").is_file() {
            let g = super::super::G2p::load(&dir, false).unwrap();
            let ps = g.phonemize("Zorbify the numbers.");
            eprintln!("{ps}");
            assert!(ps.starts_with("zˈɔɹb") && ps.contains("fˌI"), "{ps}");
        }
    }
}
