//! Try the visual review on a video folder without the GUI: draws the review frames, writes
//! the system prompt and the review prompt next to them.
//!
//! ```sh
//! cargo run -p studio-app --example review_demo -- <video folder>
//! ```
use std::path::Path;

use studio_agent::prompt::{self, ReviewFrame};
use studio_render::preview::{OpenScene, review_frames};
use studio_render::{Output, slides};

fn main() -> anyhow::Result<()> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: review_demo <video folder>"));
    let ffmpeg = studio_render::ffmpeg_path().expect("ffmpeg");
    let out = dir.join(".cache").join("review");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out)?;
    let theme_json = studio_render::style::json(0).to_string();
    let mut frames = Vec::new();
    for (i, file) in studio_render::sample::scene_files(&dir).into_iter().enumerate() {
        let req = OpenScene { path: file.clone(), root: dir.clone(), theme_json: theme_json.clone(), scene: None, globals: vec![], narration: None, narration_lines: vec![] };
        for (k, (at, frame)) in review_frames(&req, Output::new(1280, 720, 30.0))?.into_iter().enumerate() {
            let img = out.join(format!("{:02}-{}.jpg", i + 1, k + 1));
            slides::frame_to_jpeg(&ffmpeg, &frame, &img)?;
            let rel = |p: &Path| p.strip_prefix(&dir).unwrap_or(p).to_string_lossy().replace('\\', "/");
            frames.push(ReviewFrame { image: rel(&img), file: rel(&file), chapter: i + 1, at });
        }
    }
    std::fs::write(dir.join("review-prompt.txt"), prompt::review(&frames))?;
    std::fs::write(dir.join("review-system.md"), prompt::build_system(&prompt::api_digest(studio_render::TYPES)))?;
    println!("{} frames", frames.len());
    Ok(())
}
