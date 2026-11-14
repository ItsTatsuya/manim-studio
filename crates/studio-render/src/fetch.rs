//! Download one file out of a large release zip without downloading the zip.
//!
//! Release zips (ONNX Runtime, ffmpeg) carry headers, debug symbols and extra tools we
//! don't need. Each entry we want is pinned by its offset and size in the zip, so one
//! HTTP range request fetches just its local header and deflated bytes, which are
//! inflated straight to disk and checked against a pinned SHA-256.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

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
    /// chunk of the download as it arrives.
    pub fn download(&self, dest: &Path, mut progress: impl FnMut(u64)) -> Result<()> {
        // Local header (30 bytes) plus room for its name and extra field.
        const HEADER_ROOM: u64 = 512;
        let range = format!("bytes={}-{}", self.offset, self.offset + HEADER_ROOM + self.compressed - 1);
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let resp = ureq::get(self.url).set("Range", &range).call().with_context(|| format!("downloading {name}"))?;
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
