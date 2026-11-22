//! The helpers in a video's `lib/` folder. `kit.ts` (colours, cards) is the video's own once
//! written. Studio keeps the files the prompt and the layout check rely on up to date:
//! `layout.ts` (the regions content is placed through) and `lesson.ts` (the pieces lessons
//! are made of).

use std::path::Path;

/// Colours and cards. Written once into a new video; scenes and the assistant may change it.
pub const KIT: &str = include_str!("../samples/area-of-a-circle/lib/kit.ts");

/// The layout regions.
pub const LAYOUT: &str = include_str!("../kit/layout.ts");

/// The lesson pieces (EquationCard, Steps, Plot...).
pub const LESSON: &str = include_str!("../kit/lesson.ts");

/// Files Studio keeps current: name in `lib/`, contents, and how every version starts (so
/// Studio knows the file is its own; a rewritten one without that line is left alone).
const MANAGED: [(&str, &str, &str); 2] = [("layout.ts", LAYOUT, "// Studio layout v"), ("lesson.ts", LESSON, "// Studio lesson kit v")];

/// Write the helpers into a new video folder.
pub fn install(video_dir: &Path) -> std::io::Result<()> {
    let lib = video_dir.join("lib");
    std::fs::create_dir_all(&lib)?;
    std::fs::write(lib.join("kit.ts"), KIT)?;
    for (name, body, _) in MANAGED {
        std::fs::write(lib.join(name), body)?;
    }
    Ok(())
}

/// Bring the managed files up to date: added to videos made before they existed, replaced
/// when an older version is there.
pub fn refresh(video_dir: &Path) -> std::io::Result<()> {
    let lib = video_dir.join("lib");
    for (name, body, header) in MANAGED {
        let path = lib.join(name);
        match std::fs::read_to_string(&path) {
            Ok(s) if s.replace("\r\n", "\n") == body || !s.starts_with(header) => {}
            _ => {
                std::fs::create_dir_all(&lib)?;
                std::fs::write(&path, body)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_files_are_added_and_kept_current() {
        let dir = std::env::temp_dir().join(format!("studio-kit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let layout = dir.join("lib/layout.ts");
        refresh(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&layout).unwrap(), LAYOUT, "added to an older video");
        assert_eq!(std::fs::read_to_string(dir.join("lib/lesson.ts")).unwrap(), LESSON);
        std::fs::write(&layout, "// Studio layout v0\nold").unwrap();
        refresh(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&layout).unwrap(), LAYOUT, "an older version is replaced");
        std::fs::write(&layout, "// mine\n").unwrap();
        refresh(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&layout).unwrap(), "// mine\n", "a rewritten file is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
