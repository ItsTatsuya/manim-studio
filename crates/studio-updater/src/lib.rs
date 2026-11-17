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

use std::cmp::Ordering;
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

/// Semver precedence of two versions (a "v" prefix and `+build` metadata are ignored, a
/// missing component counts as 0): "0.10.0" > "0.9.3"; a release outranks its
/// pre-releases ("1.0.0" > "1.0.0-beta.2"); pre-release identifiers compare numerically
/// when both are numbers ("beta.10" > "beta.2"), numbers sort below words, and a longer
/// identifier list wins when the shared ones are equal.
pub fn compare(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.trim().trim_start_matches('v');
        let v = v.split_once('+').map_or(v, |(c, _)| c);
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), pre)
    }
    fn ident(a: &str, b: &str) -> Ordering {
        let num = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        match (num(a), num(b)) {
            // Digit strings compare by length (after leading zeros), then text: no overflow.
            (true, true) => {
                let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
                a.len().cmp(&b.len()).then_with(|| a.cmp(b))
            }
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => a.cmp(b),
        }
    }
    let (mut pa, prea) = split(a);
    let (mut pb, preb) = split(b);
    let n = pa.len().max(pb.len());
    pa.resize(n, 0);
    pb.resize(n, 0);
    pa.cmp(&pb).then_with(|| match (prea, preb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let (mut xs, mut ys) = (x.split('.'), y.split('.'));
            loop {
                match (xs.next(), ys.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(p), Some(q)) => match ident(p, q) {
                        Ordering::Equal => {}
                        o => return o,
                    },
                }
            }
        }
    })
}

/// `a > b` by [`compare`] ("0.10.0" > "0.9.3", "1.0.0" > "1.0.0-beta.2").
pub fn newer(a: &str, b: &str) -> bool {
    compare(a, b) == Ordering::Greater
}

/// Check a signed manifest. `Ok(None)` when it isn't newer than `current`.
pub fn verify_manifest(manifest: &[u8], signature: &str, public_key: &str, current: &str) -> Result<Option<Manifest>> {
    let pk = minisign_verify::PublicKey::from_base64(public_key.trim()).map_err(|e| anyhow::anyhow!("bad update key: {e}"))?;
    let sig = minisign_verify::Signature::decode(signature).map_err(|e| anyhow::anyhow!("bad manifest signature: {e}"))?;
    pk.verify(manifest, &sig, false).map_err(|_| anyhow::anyhow!("the update manifest's signature doesn't match"))?;
    let m: Manifest = serde_json::from_slice(manifest).context("reading the update manifest")?;
    Ok(newer(&m.version, current).then_some(m))
}

/// Manifests and signatures are tiny; the app zip is a few hundred MB at most.
const MANIFEST_CAP: u64 = 1024 * 1024;
const ASSET_CAP: u64 = 512 * 1024 * 1024;

/// A GET with connect and idle (per-read) timeouts, so a slow link can still finish a
/// large download but a stalled one fails. (ureq's `timeout` is a whole-request deadline.)
fn get(url: &str, cap: u64) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(30))
        .timeout_write(std::time::Duration::from_secs(30))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", concat!("manim-studio-updater/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => anyhow::anyhow!("no release has been published yet"),
            e => anyhow::anyhow!("{e}"),
        })?;
    let mut buf = Vec::new();
    resp.into_reader().take(cap).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Fetch and verify the channel's manifest (`<base>/manifest-<channel>.json` + `.minisig`).
pub fn check(base_url: &str, channel: &str, public_key: &str, current: &str) -> Result<Option<Manifest>> {
    let url = format!("{}/manifest-{channel}.json", base_url.trim_end_matches('/'));
    let manifest = get(&url, MANIFEST_CAP)?;
    let sig = String::from_utf8(get(&format!("{url}.minisig"), MANIFEST_CAP)?).context("signature is not text")?;
    verify_manifest(&manifest, &sig, public_key, current)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex(&h.finalize()))
}

/// Versions become folder names: only what a version string can contain.
fn valid_version(v: &str) -> bool {
    !v.is_empty() && v.len() < 64 && !v.starts_with('.') && v.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'+'))
}

/// The `.complete` marker: the version, then `<sha256>  <file>` per staged file, so
/// `apply` can tell a damaged staging folder from a good one. `None` without a marker.
fn read_marker(dir: &Path) -> Option<std::collections::HashMap<String, String>> {
    let text = std::fs::read_to_string(dir.join(".complete")).ok()?;
    Some(text.lines().skip(1).filter_map(|l| l.split_once("  ")).map(|(h, f)| (f.to_string(), h.to_string())).collect())
}

/// Unpack a verified asset (a zip of the app's files) into `staging/<version>/`.
///
/// The files are extracted into a unique `.tmp-*` sibling folder and renamed into place
/// once complete, so two app instances staging at once can't interleave and a crash never
/// leaves a half-filled `<version>` folder.
pub fn stage(bytes: &[u8], asset: &Asset, staging: &Path, version: &str) -> Result<PathBuf> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if !valid_version(version) {
        bail!("the update has an invalid version");
    }
    if !hex(&Sha256::digest(bytes)).eq_ignore_ascii_case(asset.sha256.trim()) {
        bail!("the download doesn't match the signed checksum");
    }
    std::fs::create_dir_all(staging)?;
    let dir = staging.join(version);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = staging.join(format!(".tmp-{version}-{}-{nanos}-{n}", std::process::id()));
    let result = extract(bytes, &tmp, version).and_then(|()| install_dir_atomically(&tmp, &dir));
    if result.is_err() || tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    result?;
    Ok(dir)
}

fn extract(bytes: &[u8], tmp: &Path, version: &str) -> Result<()> {
    std::fs::create_dir_all(tmp)?;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("the update is not a zip")?;
    let mut marker = format!("{version}\n");
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let Some(rel) = f.enclosed_name() else { bail!("the update contains an unsafe path") };
        let out = tmp.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut w = std::fs::File::create(&out)?;
        let mut h = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            std::io::Write::write_all(&mut w, &buf[..n])?;
        }
        let name = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
        marker.push_str(&format!("{}  {name}\n", hex(&h.finalize())));
    }
    std::fs::write(tmp.join(".complete"), marker)?;
    Ok(())
}

/// Rename `tmp` to `dir`. A leftover `dir` that another instance completed is kept
/// (its files are hash-checked on apply); an incomplete one is replaced.
fn install_dir_atomically(tmp: &Path, dir: &Path) -> Result<()> {
    if std::fs::rename(tmp, dir).is_ok() {
        return Ok(());
    }
    if dir.join(".complete").is_file() {
        let _ = std::fs::remove_dir_all(tmp);
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(tmp, dir).with_context(|| format!("couldn't stage the update in {}", dir.display()))
}

/// Download, verify and stage the manifest's asset for this platform.
pub fn download(m: &Manifest, staging: &Path) -> Result<PathBuf> {
    let asset = m.asset().with_context(|| format!("this release has no build for {}", platform()))?;
    let bytes = get(&asset.url, ASSET_CAP)?;
    stage(&bytes, asset, staging, &m.version)
}

/// Delete staged leftovers: a `.tmp-*` folder older than an hour (a crashed extraction),
/// and version folders at or below `current` (already applied or superseded) or that
/// never got their `.complete` marker.
fn prune(staging: &Path, current: &str) {
    let hour = std::time::Duration::from_secs(3600);
    for e in std::fs::read_dir(staging).into_iter().flatten().flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !path.is_dir() {
            continue;
        }
        let stale = if name.starts_with(".tmp-") {
            e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > hour)
        } else if name.starts_with(|c: char| c.is_ascii_digit() || c == 'v') {
            !path.join(".complete").is_file() || !newer(&name, current)
        } else {
            false
        };
        if stale {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// A staged, complete update newer than `current`, if one is waiting. Also deletes
/// staged leftovers that can no longer be applied.
pub fn pending(staging: &Path, current: &str) -> Option<(String, PathBuf)> {
    prune(staging, current);
    std::fs::read_dir(staging)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(".complete").is_file())
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), p)))
        .filter(|(v, _)| !v.starts_with('.') && newer(v, current))
        .max_by(|a, b| compare(&a.0, &b.0))
}

fn old_name(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Swap the staged files into `install_dir`, keeping the current ones as `*.old`.
/// The staged files are checked against the hashes recorded when staging; a damaged
/// folder is deleted (so the next check downloads it again) and reported as an error.
pub fn apply(staged: &Path, install_dir: &Path) -> Result<()> {
    let files: Vec<PathBuf> = std::fs::read_dir(staged)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n != ".complete"))
        .collect();
    // A marker without hashes (older builds) is accepted as before; no marker is not.
    let Some(hashes) = read_marker(staged) else { bail!("the staged update is incomplete") };
    // With hashes, the folder must match the marker exactly: every listed file present
    // and intact, nothing unlisted (a quarantined or added file would mix versions).
    let damaged = |name: &str| -> Result<()> {
        let _ = std::fs::remove_dir_all(staged);
        bail!("the staged update is damaged ({name} doesn't match); it will be downloaded again")
    };
    if !hashes.is_empty() {
        // apply() installs top-level files only, so only those must be present.
        for name in hashes.keys().filter(|n| !n.contains('/')) {
            if !files.iter().any(|f| f.file_name().is_some_and(|n| n.to_string_lossy() == name.as_str())) {
                return damaged(name);
            }
        }
    }
    for src in &files {
        let name = src.file_name().unwrap().to_string_lossy().into_owned();
        let ok = match hashes.get(&name) {
            Some(want) => hash_file(src).is_ok_and(|got| got.eq_ignore_ascii_case(want)),
            None => hashes.is_empty(),
        };
        if !ok {
            return damaged(&name);
        }
    }
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

    #[test]
    fn orders_pre_releases() {
        assert!(newer("1.1.0-beta.2", "1.1.0-beta.1"));
        assert!(!newer("1.1.0-beta.1", "1.1.0-beta.2"));
        assert!(newer("1.1.0-beta.10", "1.1.0-beta.2"), "numeric identifiers compare as numbers");
        assert!(newer("1.1.0", "1.1.0-beta.9"));
        assert!(newer("1.1.0-rc.1", "1.1.0-beta.9"), "words compare lexically");
        assert!(newer("1.1.0-beta", "1.1.0-1"), "numbers sort below words");
        assert!(newer("1.1.0-beta.1", "1.1.0-beta"), "more identifiers win when the rest is equal");
        assert!(newer("1.1.1-alpha", "1.1.0"), "the core wins over the pre-release");
        assert_eq!(compare("1.0.0", "v1.0.0"), Ordering::Equal);
        assert_eq!(compare("1.0.0+build5", "1.0.0"), Ordering::Equal);
        assert_eq!(compare("1.0.0-beta.1", "1.0.0-beta.1"), Ordering::Equal);
        // Huge numeric identifiers don't overflow.
        assert!(newer("1.0.0-99999999999999999999999", "1.0.0-9999999999999999999999"));
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

    fn asset_for(zip: &[u8]) -> Asset {
        Asset { url: "unused".into(), sha256: hex(&Sha256::digest(zip)), size: zip.len() as u64 }
    }

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("studio-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn signed_update_end_to_end() {
        let tmp = temp("e2e");
        let install = tmp.join("app");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("manim-studio.exe"), b"v1").unwrap();

        let zip = zip_of(&[("manim-studio.exe", b"v2"), ("studio-engine.exe", b"engine2")]);
        let mut assets = std::collections::BTreeMap::new();
        assets.insert(platform(), asset_for(&zip));
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

    #[test]
    fn staging_is_atomic_and_prunes() {
        let tmp = temp("stage");
        let staging = tmp.join("staging");
        let zip = zip_of(&[("manim-studio.exe", b"v2")]);
        let asset = asset_for(&zip);

        // A rejected download leaves nothing behind, not even a temp folder.
        assert!(stage(b"nope", &asset, &staging, "1.2.0").is_err());
        assert!(std::fs::read_dir(&staging).map_or(true, |mut d| d.next().is_none()));
        // A zip with a bad path fails cleanly too.
        let bad_version = stage(&zip, &asset, &staging, "../evil");
        assert!(bad_version.is_err());

        // Staging twice (two instances) is fine and ends with one complete folder.
        let a = stage(&zip, &asset, &staging, "1.2.0").unwrap();
        let b = stage(&zip, &asset, &staging, "1.2.0").unwrap();
        assert_eq!(a, b);
        let names: Vec<_> = std::fs::read_dir(&staging).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["1.2.0".to_string()]);

        // An unfinished folder and an old version are pruned; the newer complete one stays.
        std::fs::create_dir_all(staging.join("1.3.0")).unwrap();
        std::fs::create_dir_all(staging.join("0.9.0")).unwrap();
        std::fs::write(staging.join("0.9.0").join(".complete"), "0.9.0").unwrap();
        assert_eq!(pending(&staging, "1.1.0").map(|p| p.0), Some("1.2.0".into()));
        assert!(!staging.join("1.3.0").exists());
        assert!(!staging.join("0.9.0").exists());
        // Once the app is at that version the folder goes too.
        assert!(pending(&staging, "1.2.0").is_none());
        assert!(!staging.join("1.2.0").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn picks_the_newest_pending_including_pre_releases() {
        let tmp = temp("pick");
        let staging = tmp.join("staging");
        let zip = zip_of(&[("a.exe", b"x")]);
        let asset = asset_for(&zip);
        stage(&zip, &asset, &staging, "1.1.0-beta.2").unwrap();
        stage(&zip, &asset, &staging, "1.1.0-beta.10").unwrap();
        assert_eq!(pending(&staging, "1.0.0").map(|p| p.0), Some("1.1.0-beta.10".into()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn apply_rejects_a_damaged_staging_folder() {
        let tmp = temp("damaged");
        let install = tmp.join("app");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("manim-studio.exe"), b"v1").unwrap();
        let staging = tmp.join("staging");
        let zip = zip_of(&[("manim-studio.exe", b"v2")]);
        let staged = stage(&zip, &asset_for(&zip), &staging, "0.2.0").unwrap();
        std::fs::write(staged.join("manim-studio.exe"), b"tampered").unwrap();
        assert!(apply(&staged, &install).is_err());
        assert_eq!(std::fs::read(install.join("manim-studio.exe")).unwrap(), b"v1", "the install is untouched");
        assert!(!staged.exists(), "the damaged folder is dropped so it downloads again");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn apply_rejects_a_missing_or_unlisted_file() {
        let tmp = temp("missing");
        let install = tmp.join("app");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("manim-studio.exe"), b"v1").unwrap();
        std::fs::write(install.join("studio-engine.exe"), b"e1").unwrap();
        let staging = tmp.join("staging");
        let zip = zip_of(&[("manim-studio.exe", b"v2"), ("studio-engine.exe", b"e2")]);
        // A listed file went missing (e.g. quarantined): don't install a mixed version.
        let staged = stage(&zip, &asset_for(&zip), &staging, "0.2.0").unwrap();
        std::fs::remove_file(staged.join("studio-engine.exe")).unwrap();
        assert!(apply(&staged, &install).is_err());
        assert_eq!(std::fs::read(install.join("manim-studio.exe")).unwrap(), b"v1");
        // A file nobody listed appeared.
        let staged = stage(&zip, &asset_for(&zip), &staging, "0.2.0").unwrap();
        std::fs::write(staged.join("extra.dll"), b"x").unwrap();
        assert!(apply(&staged, &install).is_err());
        assert_eq!(std::fs::read(install.join("studio-engine.exe")).unwrap(), b"e1");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
