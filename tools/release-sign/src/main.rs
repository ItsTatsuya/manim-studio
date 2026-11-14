//! Update signing for Manim Studio releases, in minisign's format.
//!
//! ```text
//! release-sign keygen <dir>          # writes <dir>/manim-studio.key (secret) and .pub
//! release-sign sign <file>           # writes <file>.minisig
//! ```
//!
//! `sign` reads the secret key from `MINISIGN_SECRET` (the key file's contents, as CI
//! stores it) or from the file named by `MINISIGN_KEY`. Keys are unencrypted: keep the
//! secret key file out of the repository and backed up.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use minisign::{KeyPair, PublicKey, SecretKey, SecretKeyBox};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen", dir] => keygen(Path::new(dir)),
        ["sign", file] => sign(Path::new(file)),
        _ => bail!("usage: release-sign keygen <dir> | release-sign sign <file>"),
    }
}

fn keygen(dir: &Path) -> Result<()> {
    let sk_path = dir.join("manim-studio.key");
    let pk_path = dir.join("manim-studio.pub");
    if sk_path.exists() || pk_path.exists() {
        bail!("{} already has a key; refusing to replace it", dir.display());
    }
    fs::create_dir_all(dir)?;
    // An empty password stores the key unencrypted but with its checksum (like `minisign -W`).
    let kp = KeyPair::generate_encrypted_keypair(Some(String::new()))?;
    let sk_box = kp.sk.to_box(Some("Manim Studio update signing key (secret)"))?;
    fs::write(&sk_path, sk_box.into_string())?;
    fs::write(&pk_path, kp.pk.to_box()?.into_string())?;
    println!("secret key: {}", sk_path.display());
    println!("public key: {}", pk_path.display());
    println!("STUDIO_UPDATE_PUBKEY={}", kp.pk.to_base64());
    Ok(())
}

fn secret_key() -> Result<SecretKey> {
    let text = match std::env::var("MINISIGN_SECRET") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => {
            let path: PathBuf = std::env::var_os("MINISIGN_KEY")
                .context("set MINISIGN_SECRET (key contents) or MINISIGN_KEY (key file)")?
                .into();
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
        }
    };
    // Unencrypted keys load with an empty password (None would prompt).
    SecretKey::from_box(SecretKeyBox::from_string(text.trim())?, Some(String::new())).context("reading the secret key")
}

fn sign(file: &Path) -> Result<()> {
    let sk = secret_key()?;
    let pk = PublicKey::from_secret_key(&sk)?;
    let data = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let sig = minisign::sign(Some(&pk), &sk, Cursor::new(&data), None, None)?;
    let out = PathBuf::from(format!("{}.minisig", file.display()));
    fs::write(&out, sig.to_string())?;
    println!("signed: {} (key {})", out.display(), pk.to_base64());
    Ok(())
}
