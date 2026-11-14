//! The bundled sample lesson ("Area of a circle"), written to disk on first run so the
//! studio has a real video to preview and render before the assistant exists.
//!
//! A video on disk is a folder: `scenes/*.ts` (one scene per file, in order),
//! `lib/` (shared helpers), `assets/`, plus `.cache/` and `exports/` written by the
//! studio.

use std::path::{Path, PathBuf};

/// Files of the sample, relative to the video folder.
pub const AREA_OF_A_CIRCLE: [(&str, &str); 6] = [
    ("lib/kit.ts", include_str!("../samples/area-of-a-circle/lib/kit.ts")),
    ("scenes/01-hook.ts", include_str!("../samples/area-of-a-circle/scenes/01-hook.ts")),
    ("scenes/02-growth.ts", include_str!("../samples/area-of-a-circle/scenes/02-growth.ts")),
    ("scenes/03-unroll.ts", include_str!("../samples/area-of-a-circle/scenes/03-unroll.ts")),
    ("scenes/04-formula.ts", include_str!("../samples/area-of-a-circle/scenes/04-formula.ts")),
    ("scenes/05-practice.ts", include_str!("../samples/area-of-a-circle/scenes/05-practice.ts")),
];

/// The sample's `video.json` (title, storyboard). Written only when missing: the studio
/// keeps the video's status and settings in it.
pub const VIDEO_JSON: &str = include_str!("../samples/area-of-a-circle/video.json");

/// Bumped whenever the bundled files change, so an old copy is refreshed.
const VERSION: &str = "3";

/// Write the sample into `dir` unless an up-to-date copy is there. Files the user
/// edited are kept when the version matches. Returns the scene scripts in order.
pub fn install(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let marker = dir.join(".sample-version");
    let current = std::fs::read_to_string(&marker).is_ok_and(|v| v.trim() == VERSION);
    for (rel, body) in AREA_OF_A_CIRCLE {
        let path = dir.join(rel);
        if current && path.is_file() {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, body)?;
    }
    std::fs::create_dir_all(dir.join("assets"))?;
    merge_video_json(&dir.join("video.json"), !current)?;
    std::fs::write(&marker, VERSION)?;
    Ok(scene_files(dir))
}

/// Scene scripts of a video folder, in file-name order.
pub fn scene_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir.join("scenes"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && crate::preview::is_script(p))
        .collect();
    v.sort();
    v
}

/// Add the sample's title and storyboard to `video.json`, keeping fields the studio wrote
/// (status, settings, session).
/// Add missing keys from the bundled `video.json`; on a new sample version also take its
/// storyboard (the narration is part of the sample).
fn merge_video_json(path: &Path, upgrade: bool) -> std::io::Result<()> {
    let bundled: serde_json::Value = serde_json::from_str(VIDEO_JSON).expect("bundled video.json is valid");
    let mut current: serde_json::Value = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let mut changed = false;
    if let (Some(cur), Some(b)) = (current.as_object_mut(), bundled.as_object()) {
        for (k, v) in b {
            if !cur.contains_key(k) || (upgrade && k == "storyboard" && cur.get(k) != Some(v)) {
                cur.insert(k.clone(), v.clone());
                changed = true;
            }
        }
    }
    if !changed {
        return Ok(());
    }
    std::fs::write(path, serde_json::to_vec_pretty(&current).unwrap_or_default())
}
