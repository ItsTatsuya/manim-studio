//! Fonts the engine should know besides the system ones (the app's bundled Inter and
//! JetBrains Mono), so lessons look the same on every machine.

use std::path::Path;

/// Register font files' bytes with the engine (process-wide). Errors are ignored: a
/// missing font only means the engine falls back to a system face.
pub fn register_data<'a>(fonts: impl IntoIterator<Item = &'a [u8]>) {
    for bytes in fonts {
        let _ = manim::text::register_font_data(bytes.to_vec());
    }
}

/// Register every font in a folder (for the engine worker: `--fonts DIR`).
pub fn register_dir(dir: &Path) {
    let _ = manim::text::register_font_dir(dir);
}

/// Write font files into `dir` once, so engine worker processes can load them.
pub fn materialize<'a>(dir: &Path, fonts: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, bytes) in fonts {
        let path = dir.join(name);
        let same = std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64);
        if !same {
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(tmp, &path)?;
        }
    }
    Ok(())
}
