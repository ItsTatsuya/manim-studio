//! The helpers in a video's `lib/` folder: `kit.ts` (colours, cards) and `layout.ts` (the
//! regions content is placed through). `kit.ts` is the video's own once written; Studio
//! keeps `layout.ts` up to date, since the layout check depends on it.

use std::path::Path;

/// Colours and cards. Written once into a new video; scenes and the assistant may change it.
pub const KIT: &str = include_str!("../samples/area-of-a-circle/lib/kit.ts");

/// The layout regions. Its first line names the version.
pub const LAYOUT: &str = include_str!("../kit/layout.ts");

/// How every version of `layout.ts` starts, so Studio knows the file is its own.
const LAYOUT_HEADER: &str = "// Studio layout v";

/// Write the helpers into a new video folder.
pub fn install(video_dir: &Path) -> std::io::Result<()> {
    let lib = video_dir.join("lib");
    std::fs::create_dir_all(&lib)?;
    std::fs::write(lib.join("kit.ts"), KIT)?;
    std::fs::write(lib.join("layout.ts"), LAYOUT)
}

/// Bring `lib/layout.ts` up to date: added to videos made before it existed, replaced when
/// an older version is there. A file someone rewrote (no version line) is left alone.
pub fn refresh(video_dir: &Path) -> std::io::Result<()> {
    let path = video_dir.join("lib").join("layout.ts");
    match std::fs::read_to_string(&path) {
        Ok(s) if s.replace("\r\n", "\n") == LAYOUT || !s.starts_with(LAYOUT_HEADER) => Ok(()),
        _ => {
            std::fs::create_dir_all(video_dir.join("lib"))?;
            std::fs::write(&path, LAYOUT)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_added_and_kept_current() {
        let dir = std::env::temp_dir().join(format!("studio-kit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let layout = dir.join("lib/layout.ts");
        refresh(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&layout).unwrap(), LAYOUT, "added to an older video");
        std::fs::write(&layout, format!("{LAYOUT_HEADER}0\nold")).unwrap();
        refresh(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&layout).unwrap(), LAYOUT, "an older version is replaced");
        std::fs::write(&layout, "// mine\n").unwrap();
        refresh(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&layout).unwrap(), "// mine\n", "a rewritten file is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
