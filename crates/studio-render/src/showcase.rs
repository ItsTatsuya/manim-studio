//! The style showcase: a short title-card scene the studio renders once per house style
//! for the style pickers (a still plus a looping clip). It reads everything from the
//! theme, so one script shows every style.

use std::path::{Path, PathBuf};

const SCRIPT: &str = include_str!("../samples/style-showcase/showcase.ts");

/// Write the showcase script into `dir` (when missing or out of date); returns its path.
pub fn install(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("showcase.ts");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(SCRIPT) {
        std::fs::write(&path, SCRIPT)?;
    }
    Ok(path)
}
