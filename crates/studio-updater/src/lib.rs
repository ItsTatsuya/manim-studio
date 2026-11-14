//! Updates for Manim Studio (DESIGN.md §7).
//!
//! A release publishes, per channel, a JSON manifest and its minisign signature:
//!
//! ```json
//! { "version": "0.2.0", "notes": "…", "assets": { "windows-x86_64": { "url": "…zip", "sha256": "…", "size": 123 } } }
//! ```
//!
//! The app verifies the manifest signature with the public key built into it, downloads
//! the asset for its platform in the background, checks its SHA-256 against the signed
//! manifest, and unpacks it into a staging folder. Applying swaps the files on the next
//! start (or "Restart now"): each running file is renamed to `*.old` (Windows allows
//! renaming a running executable), the new file takes its place, and the old files stay
//! for rollback.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    pub assets: std::collections::BTreeMap<String, Asset>,
}

impl Manifest {
    /// The asset for this computer ("windows-x86_64", "macos-aarch64", ...).
    pub fn asset(&self) -> Option<&Asset> {
        self.assets.get(&platform())
    }
}

pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// `a > b` for dotted versions ("0.10.0" > "0.9.3"); pre-release suffixes sort first.
pub fn newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v, false),
        };
        (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), pre)
    }
    let (mut pa, prea) = parts(a);
    let (mut pb, preb) = parts(b);
    let n = pa.len().max(pb.len());
    pa.resize(n, 0);
    pb.resize(n, 0);
    match pa.cmp(&pb) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => preb && !prea,
    }
}

/// Check a signed manifest. `Ok(None)` when it isn't newer than `current`.
pub fn verify_manifest(manifest: &[u8], signature: &str, public_key: &str, current: &str) -> Result<Option<Manifest>> {
    let pk = minisign_verify::PublicKey::from_base64(public_key.trim()).map_err(|e| anyhow::anyhow!("bad update key: {e}"))?;
    let sig = minisign_verify::Signature::decode(signature).map_err(|e| anyhow::anyhow!("bad manifest signature: {e}"))?;
    pk.verify(manifest, &sig, false).map_err(|_| anyhow::anyhow!("the update manifest's signature doesn't match"))?;
    let m: Manifest = serde_json::from_slice(manifest).context("reading the update manifest")?;
    Ok(newer(&m.version, current).then_some(m))
}

fn get(url: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url)
        .set("User-Agent", concat!("manim-studio-updater/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => anyhow::anyhow!("no release has been published yet"),
            e => anyhow::anyhow!("{e}"),
        })?;
    let mut buf = Vec::new();
    resp.into_reader().take(512 * 1024 * 1024).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Fetch and verify the channel's manifest (`<base>/manifest-<channel>.json` + `.minisig`).
pub fn check(base_url: &str, channel: &str, public_key: &str, current: &str) -> Result<Option<Manifest>> {
    let url = format!("{}/manifest-{channel}.json", base_url.trim_end_matches('/'));
    let manifest = get(&url)?;
    let sig = String::from_utf8(get(&format!("{url}.minisig"))?).context("signature is not text")?;
    verify_manifest(&manifest, &sig, public_key, current)
}

/// Unpack a verified asset (a zip of the app's files) into `staging/<version>/`.
pub fn stage(bytes: &[u8], asset: &Asset, staging: &Path, version: &str) -> Result<PathBuf> {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if !hex.eq_ignore_ascii_case(asset.sha256.trim()) {
        bail!("the download doesn't match the signed checksum");
    }
    let dir = staging.join(version);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("the update is not a zip")?;
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let Some(rel) = f.enclosed_name() else { bail!("the update contains an unsafe path") };
        let out = dir.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut f, &mut w)?;
    }
    std::fs::write(dir.join(".complete"), version)?;
    Ok(dir)
}

/// Download, verify and stage the manifest's asset for this platform.
pub fn download(m: &Manifest, staging: &Path) -> Result<PathBuf> {
    let asset = m.asset().with_context(|| format!("this release has no build for {}", platform()))?;
    let bytes = get(&asset.url)?;
    stage(&bytes, asset, staging, &m.version)
}

/// A staged, complete update newer than `current`, if one is waiting.
pub fn pending(staging: &Path, current: &str) -> Option<(String, PathBuf)> {
    std::fs::read_dir(staging)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(".complete").is_file())
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), p)))
        .filter(|(v, _)| newer(v, current))
        .max_by(|a, b| if newer(&a.0, &b.0) { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less })
}

fn old_name(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Swap the staged files into `install_dir`, keeping the current ones as `*.old`.
pub fn apply(staged: &Path, install_dir: &Path) -> Result<()> {
    let files: Vec<PathBuf> = std::fs::read_dir(staged)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n != ".complete"))
        .collect();
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for src in &files {
        let dest = install_dir.join(src.file_name().unwrap());
        let backup = old_name(&dest);
        let _ = std::fs::remove_file(&backup);
        if dest.exists() {
            if let Err(e) = std::fs::rename(&dest, &backup) {
                // Undo what was swapped so far.
                for (d, b) in moved.iter().rev() {
                    let _ = std::fs::rename(b, d);
                }
                bail!("couldn't replace {}: {e}", dest.display());
            }
            moved.push((dest.clone(), backup));
        }
        if let Err(e) = std::fs::copy(src, &dest) {
            // Never leave the install without its files: put the old ones back.
            let _ = std::fs::remove_file(&dest);
            for (d, b) in moved.iter().rev() {
                let _ = std::fs::rename(b, d);
            }
            bail!("couldn't install {}: {e}", dest.display());
        }
    }
    let _ = std::fs::remove_dir_all(staged);
    Ok(())
}

/// Whether a previous version is kept for rollback.
pub fn can_roll_back(install_dir: &Path) -> bool {
    std::fs::read_dir(install_dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().ends_with(".old"))
}

/// Put the `*.old` files back (the current ones become `*.new`, removed next start).
pub fn roll_back(install_dir: &Path) -> Result<()> {
    for e in std::fs::read_dir(install_dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(orig) = name.strip_suffix(".old") {
            let cur = install_dir.join(orig);
            let newer = install_dir.join(format!("{orig}.new"));
            let _ = std::fs::remove_file(&newer);
            if cur.exists() {
                std::fs::rename(&cur, &newer)?;
            }
            std::fs::rename(e.path(), &cur)?;
        }
    }
    Ok(())
}

/// Remove leftovers of earlier swaps that are no longer running (`*.new`).
pub fn clean(install_dir: &Path) {
    for e in std::fs::read_dir(install_dir).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().ends_with(".new") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn compares_versions() {
        assert!(newer("0.10.0", "0.9.3"));
        assert!(newer("1.0.0", "1.0.0-beta"));
        assert!(!newer("1.0.0-beta", "1.0.0"));
        assert!(!newer("0.1.0", "0.1"));
        assert!(newer("v0.2", "0.1.9"));
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            for (name, data) in files {
                w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn signed_update_end_to_end() {
        let tmp = std::env::temp_dir().join(format!("studio-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let install = tmp.join("app");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("manim-studio.exe"), b"v1").unwrap();

        let zip = zip_of(&[("manim-studio.exe", b"v2"), ("studio-engine.exe", b"engine2")]);
        let sha: String = Sha256::digest(&zip).iter().map(|b| format!("{b:02x}")).collect();
        let mut assets = std::collections::BTreeMap::new();
        assets.insert(platform(), Asset { url: "unused".into(), sha256: sha, size: zip.len() as u64 });
        let manifest = serde_json::to_vec(&Manifest { version: "0.2.0".into(), notes: "n".into(), assets }).unwrap();

        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let sig = minisign::sign(Some(&kp.pk), &kp.sk, Cursor::new(&manifest), None, None).unwrap().to_string();
        let pk = kp.pk.to_base64();

        // Signature and version checks.
        let m = verify_manifest(&manifest, &sig, &pk, "0.1.0").unwrap().expect("newer");
        assert!(verify_manifest(&manifest, &sig, &pk, "0.2.0").unwrap().is_none());
        let mut tampered = manifest.clone();
        tampered[5] ^= 1;
        assert!(verify_manifest(&tampered, &sig, &pk, "0.1.0").is_err());

        // Checksum, staging, swap and rollback.
        let staging = tmp.join("staging");
        let asset = m.asset().unwrap();
        assert!(stage(b"not the zip", asset, &staging, "0.2.0").is_err());
        let staged = stage(&zip, asset, &staging, "0.2.0").unwrap();
        assert_eq!(pending(&staging, "0.1.0").map(|p| p.0), Some("0.2.0".into()));
        apply(&staged, &install).unwrap();
        assert_eq!(std::fs::read(install.join("manim-studio.exe")).unwrap(), b"v2");
        assert_eq!(std::fs::read(install.join("manim-studio.exe.old")).unwrap(), b"v1");
        assert!(can_roll_back(&install));
        roll_back(&install).unwrap();
        assert_eq!(std::fs::read(install.join("manim-studio.exe")).unwrap(), b"v1");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
