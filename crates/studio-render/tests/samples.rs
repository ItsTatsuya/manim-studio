//! The bundled sample lesson must load, run and lay out cleanly in every house style.

use studio_render::preview::{OpenScene, check};
use studio_render::{sample, style};

#[test]
fn sample_scenes_run_cleanly_in_every_style() {
    let dir = std::env::temp_dir().join(format!("studio-sample-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let scenes = sample::install(&dir).unwrap();
    assert_eq!(scenes.len(), 5);
    let mut failures = Vec::new();
    for (i, s) in style::STYLES.iter().enumerate() {
        for path in &scenes {
            let req = OpenScene {
                path: path.clone(),
                root: dir.clone(),
                theme_json: style::json(i).to_string(),
                scene: None,
                globals: vec![],
                narration: None,
                narration_lines: vec![],
            };
            match check(&req) {
                Ok(t) => {
                    assert!(t.duration > 5.0, "{} is too short", path.display());
                    for p in &t.problems {
                        failures.push(format!("{} [{}]: {} at {:.2}s", path.display(), s.name, p.message, p.time));
                    }
                }
                Err(e) => failures.push(format!("{} [{}]: {e:#}", path.display(), s.name)),
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Each sample scene places every sentence of its narration with `narrate(n)`.
#[test]
fn sample_scenes_place_every_sentence() {
    let video: serde_json::Value = serde_json::from_str(studio_render::sample::VIDEO_JSON).unwrap();
    let beats = video["storyboard"]["scenes"].as_array().unwrap();
    let scripts: Vec<&str> = studio_render::sample::AREA_OF_A_CIRCLE.iter().filter(|(p, _)| p.starts_with("scenes/")).map(|(_, b)| *b).collect();
    assert_eq!(beats.len(), scripts.len());
    for (beat, script) in beats.iter().zip(scripts) {
        let n = studio_render::voice::sentences(beat["say"].as_str().unwrap()).len();
        let called: std::collections::BTreeSet<usize> = script
            .match_indices("narrate(")
            .filter_map(|(i, _)| script[i + 8..].split(')').next()?.parse().ok())
            .collect();
        assert_eq!(called, (1..=n).collect(), "{}", beat["title"]);
    }
}
