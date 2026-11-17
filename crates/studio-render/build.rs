//! Derives `ENGINE_REV` from the workspace `Cargo.lock` so the render cache key follows
//! the pinned engine automatically.

fn main() {
    let lock = std::path::Path::new(&std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default()).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    let rev = std::fs::read_to_string(&lock).ok().and_then(|t| engine_sha(&t)).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=STUDIO_ENGINE_REV=manim-rust@{rev}");
}

/// The short commit of the `manim` package's git source in a Cargo.lock.
fn engine_sha(lock: &str) -> Option<String> {
    let mut in_manim = false;
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            in_manim = false;
        } else if line == "name = \"manim\"" {
            in_manim = true;
        } else if in_manim && line.starts_with("source = \"git+") {
            let sha = line.trim_end_matches('"').rsplit('#').next()?;
            if sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(sha[..7].to_string());
            }
        }
    }
    None
}
