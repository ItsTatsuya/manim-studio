//! Downloads: one file with resume and cancellation, and one file out of a large release
//! zip without downloading the zip.
//!
//! Release zips (ONNX Runtime, ffmpeg) carry headers, debug symbols and extra tools we
//! don't need. Each entry we want is pinned by its offset and size in the zip, so one
//! HTTP range request fetches just its local header and deflated bytes, which are
//! inflated straight to disk and checked against a pinned SHA-256.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::Cancelled;

/// An HTTP agent that gives up on a dead connection (30 s to connect, 30 s without
/// receiving data) instead of hanging forever, as `ureq::get`'s defaults would.
pub(crate) fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(30)).timeout_read(Duration::from_secs(30)).build()
}

/// Download `url` to `dest` and check it against `size` and `sha256` (hex).
///
/// The bytes go to `<dest>.part` first. A `.part` left by an interrupted or cancelled
/// attempt is resumed with an HTTP `Range` request when the server answers 206 (a 200
/// restarts from the beginning); the checksum always covers the whole final file.
/// `progress(n)` reports each chunk of `n` new bytes, starting with the bytes already
/// on disk when resuming. `cancel` is checked between chunks: the `.part` is kept and
/// [`Cancelled`] returned.
pub(crate) fn download_file(
    url: &str,
    dest: &Path,
    size: u64,
    sha256: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = dest.with_file_name(format!("{name}.part"));
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > size {
        let _ = std::fs::remove_file(&part);
        have = 0;
    }
    let mut hasher = Sha256::new();
    if have < size {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let agent = agent();
        let mut resp = None;
        if have > 0 {
            match agent.get(url).set("Range", &format!("bytes={have}-")).call() {
                Ok(r) if r.status() == 206 && r.header("Content-Range").is_some_and(|c| c.trim_start().starts_with(&format!("bytes {have}-"))) => {
                    resp = Some(r)
                }
                // The server ignored the range (200), sent something else, or refused it
                // (416): start over.
                Ok(_) | Err(ureq::Error::Status(416, _)) => {}
                Err(e) => return Err(e).with_context(|| format!("downloading {name}")),
            }
        }
        let resumed = resp.is_some();
        let resp = match resp {
            Some(r) => r,
            None => agent.get(url).call().with_context(|| format!("downloading {name}"))?,
        };
        if resumed {
            // Hash what is already on disk, then carry on after it.
            let mut old = std::fs::File::open(&part)?;
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = old.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            progress(have);
        } else {
            have = 0;
        }
        let mut file = if resumed { std::fs::OpenOptions::new().append(true).open(&part)? } else { std::fs::File::create(&part)? };
        let mut reader = resp.into_reader();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Cancelled.into());
            }
            let n = reader.read(&mut buf).with_context(|| format!("downloading {name}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            have += n as u64;
            progress(n as u64);
            if have > size {
                break;
            }
        }
        file.flush()?;
        drop(file);
        if have < size {
            // Cut off early: keep the bytes so the next attempt resumes.
            bail!("the download of {name} was interrupted");
        }
    } else {
        // Already complete (a finished attempt that wasn't renamed): just verify it.
        let mut old = std::fs::File::open(&part)?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = old.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        progress(have);
    }
    if have != size || format!("{:x}", hasher.finalize()) != sha256 {
        let _ = std::fs::remove_file(&part);
        bail!("{name} failed its checksum");
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

/// One file inside a remote zip, pinned to an exact release.
pub struct ZipEntry {
    pub url: &'static str,
    /// Where the entry's local file header starts in the zip.
    pub offset: u64,
    /// Deflated size in the zip (what the download transfers).
    pub compressed: u64,
    /// Inflated size and SHA-256.
    pub size: u64,
    pub sha256: &'static str,
    /// The entry's path inside the zip ends with this (a sanity check on the offset).
    pub name_suffix: &'static str,
}

impl ZipEntry {
    /// Already downloaded (the size is checked; the hash was checked on download).
    pub fn present(&self, dest: &Path) -> bool {
        std::fs::metadata(dest).is_ok_and(|m| m.len() == self.size)
    }

    /// Fetch, inflate and verify the entry into `dest`. `progress(bytes)` reports each
    /// chunk of the download as it arrives. This inflates as it goes, so an interrupted
    /// attempt can't resume: `cancel` (checked between chunks) or an error restarts it.
    /// Timeouts (30 s without data) keep a dead connection from hanging.
    pub fn download(&self, dest: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64)) -> Result<()> {
        // Local header (30 bytes) plus room for its name and extra field.
        const HEADER_ROOM: u64 = 512;
        let range = format!("bytes={}-{}", self.offset, self.offset + HEADER_ROOM + self.compressed - 1);
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let resp = agent().get(self.url).set("Range", &range).call().with_context(|| format!("downloading {name}"))?;
        if resp.status() != 206 {
            bail!("the server didn't send a partial download for {name} (status {})", resp.status());
        }
        let mut reader = resp.into_reader();

        // Zip local file header: signature, then name and extra-field lengths at 26..30.
        let mut head = [0u8; 30];
        reader.read_exact(&mut head)?;
        if head[..4] != [0x50, 0x4b, 0x03, 0x04] {
            bail!("{name}: the download wasn't the expected file");
        }
        let name_len = u16::from_le_bytes([head[26], head[27]]) as usize;
        let extra_len = u16::from_le_bytes([head[28], head[29]]) as usize;
        let mut entry_name = vec![0u8; name_len];
        reader.read_exact(&mut entry_name)?;
        if !entry_name.ends_with(self.name_suffix.as_bytes()) {
            bail!("{name}: the download wasn't the expected file");
        }
        std::io::copy(&mut (&mut reader).take(extra_len as u64), &mut std::io::sink())?;
        progress((30 + name_len + extra_len) as u64);

        // Inflate straight to a .part file, hashing as it goes.
        let counted = Counted { inner: (&mut reader).take(self.compressed), progress: &mut progress };
        let mut inflate = flate2::read::DeflateDecoder::new(counted);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let part = dest.with_file_name(format!("{name}.part"));
        let mut file = std::fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut written = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(file);
                let _ = std::fs::remove_file(&part);
                return Err(Cancelled.into());
            }
            let n = inflate.read(&mut buf).with_context(|| format!("unpacking {name}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            written += n as u64;
        }
        drop(file);
        if written != self.size || format!("{:x}", hasher.finalize()) != self.sha256 {
            let _ = std::fs::remove_file(&part);
            bail!("{name} failed its checksum");
        }
        std::fs::rename(&part, dest)?;
        Ok(())
    }
}

/// A reader that reports how many bytes pass through it.
struct Counted<'a, R, F: FnMut(u64)> {
    inner: R,
    progress: &'a mut F,
}

impl<R: Read, F: FnMut(u64)> Read for Counted<'_, R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        (self.progress)(n as u64);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    type Asked = std::sync::Arc<std::sync::Mutex<Vec<Option<u64>>>>;

    /// A one-file HTTP server (a detached thread; it ends with the test process). With
    /// `ranges` it honours `Range: bytes=N-` (206), else it always answers 200 with the
    /// whole body. `cut` drops the connection after that many body bytes on the first
    /// response (a flaky download). Returns the URL and the `Range` start of each request.
    fn serve(body: Vec<u8>, ranges: bool, cut: Option<usize>) -> (String, Asked) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
        let asked: Asked = Default::default();
        let log = asked.clone();
        std::thread::spawn(move || {
            let mut first = true;
            while let Ok((mut sock, _)) = listener.accept() {
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                let mut from = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse::<u64>().ok();
                    }
                }
                log.lock().unwrap().push(from);
                let (status, start) = match (ranges, from) {
                    (true, Some(f)) => (format!("206 Partial Content\r\nContent-Range: bytes {f}-{}/{}", body.len() - 1, body.len()), f as usize),
                    _ => ("200 OK".to_string(), 0),
                };
                let slice = &body[start..];
                let _ = write!(sock, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", slice.len());
                let send = if first { cut.map_or(slice.len(), |c| c.min(slice.len())) } else { slice.len() };
                let _ = sock.write_all(&slice[..send]);
                first = false;
            }
        });
        (url, asked)
    }

    fn sha(b: &[u8]) -> String {
        format!("{:x}", Sha256::digest(b))
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("studio-fetch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn resumes_a_cut_download_with_a_range_request() {
        let body: Vec<u8> = (0..400_000u32).map(|i| (i % 253) as u8).collect();
        let d = dir("resume");
        let dest = d.join("file.bin");
        let (url, asked) = serve(body.clone(), true, Some(150_000));
        let cancel = AtomicBool::new(false);
        let first = download_file(&url, &dest, body.len() as u64, &sha(&body), &cancel, |_| {});
        assert!(first.is_err() && !dest.exists());
        let kept = std::fs::metadata(d.join("file.bin.part")).unwrap().len();
        assert!(kept > 0 && kept < body.len() as u64);
        let mut total = 0u64;
        download_file(&url, &dest, body.len() as u64, &sha(&body), &cancel, |n| total += n).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(total, body.len() as u64, "progress counts the resumed bytes too");
        assert_eq!(*asked.lock().unwrap(), vec![None, Some(kept)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restarts_when_the_server_ignores_ranges_and_checks_the_hash() {
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 241) as u8).collect();
        let d = dir("restart");
        let dest = d.join("file.bin");
        std::fs::write(d.join("file.bin.part"), &body[..30_000]).unwrap();
        let (url, _) = serve(body.clone(), false, None);
        download_file(&url, &dest, body.len() as u64, &sha(&body), &AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        // A wrong hash is refused and leaves nothing behind.
        let dest2 = d.join("other.bin");
        let e = download_file(&url, &dest2, body.len() as u64, &sha(b"nope"), &AtomicBool::new(false), |_| {}).unwrap_err();
        assert!(e.to_string().contains("checksum"), "{e}");
        assert!(!dest2.exists() && !d.join("other.bin.part").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

}
