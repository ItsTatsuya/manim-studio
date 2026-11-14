//! Programs Studio downloads on first use instead of shipping in the installer: ffmpeg
//! (MP4 encoding, joining scenes, adding narration).
//!
//! ffmpeg is the gyan.dev "essentials" build (GPL; it includes libx264, AAC and the
//! NVENC / AMF / QSV encoders). It runs as a separate program and is fetched from its
//! publisher, with its licence saved next to it. Only `ffmpeg.exe` is taken out of the
//! release zip (about 37 MB of 115 MB).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;

use crate::fetch::ZipEntry;

const ZIP: &str = "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip";

const FFMPEG: ZipEntry = ZipEntry {
    url: ZIP,
    offset: 124,
    compressed: 37_386_196,
    size: 105_423_872,
    sha256: "3256173f3f8bffd7df12227c68adf68025edb1832273a9530688a7bb1ed8edec",
    name_suffix: "bin/ffmpeg.exe",
};

const LICENSE: ZipEntry = ZipEntry {
    url: ZIP,
    offset: 114_739_506,
    compressed: 11_559,
    size: 35_147,
    sha256: "8ceb4b9ee5adedde47b31e975c1d90c73ad27b6b165a1dcd80c7c545eb65b903",
    name_suffix: "/LICENSE",
};

/// The ffmpeg version this downloads, for Settings.
pub const FFMPEG_VERSION: &str = "9.0.2";

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where downloaded tools live (`<data>/tools`). Set once at startup.
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

fn ffmpeg_dir() -> Option<PathBuf> {
    DIR.get().map(|d| d.join("ffmpeg"))
}

/// The downloaded ffmpeg, when it's there.
pub fn ffmpeg() -> Option<PathBuf> {
    let exe = ffmpeg_dir()?.join("ffmpeg.exe");
    (cfg!(windows) && FFMPEG.present(&exe)).then_some(exe)
}

/// Bytes a download transfers (ffmpeg and its licence).
pub fn ffmpeg_download_size() -> u64 {
    FFMPEG.compressed + LICENSE.compressed
}

/// Download ffmpeg (and its licence) for this computer. `progress(done, total)`.
pub fn download_ffmpeg(mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let Some(dir) = ffmpeg_dir() else { anyhow::bail!("no data folder for ffmpeg") };
    if !cfg!(windows) {
        anyhow::bail!("install ffmpeg with your package manager");
    }
    let total = ffmpeg_download_size();
    let mut done = 0u64;
    for (entry, file) in [(&LICENSE, "LICENSE.txt"), (&FFMPEG, "ffmpeg.exe")] {
        let dest: &Path = &dir.join(file);
        if entry.present(dest) {
            done += entry.compressed;
            continue;
        }
        entry.download(dest, |n| {
            done = (done + n).min(total);
            progress(done, total);
        })?;
    }
    progress(total, total);
    Ok(())
}

#[cfg(test)]
mod tests {
    /// `FFMPEG_DOWNLOAD=<dir>`: fetch ffmpeg there and check it runs.
    #[test]
    fn downloads_ffmpeg() {
        let Some(d) = std::env::var_os("FFMPEG_DOWNLOAD") else { return };
        super::set_dir(std::path::PathBuf::from(d));
        super::download_ffmpeg(|done, total| assert!(done <= total)).unwrap();
        let exe = super::ffmpeg().expect("downloaded");
        let out = std::process::Command::new(&exe).args(["-hide_banner", "-encoders"]).output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("libx264"));
    }
}
