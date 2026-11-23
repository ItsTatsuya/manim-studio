//! Try the look pass on a video folder without the GUI: draws the contact sheets (stills at
//! the end of each cue of every chapter), writes the system prompt and the look prompt next to
//! them, and lists the stills with their times and script lines.
//!
//! ```sh
//! cargo run -p studio-app --example review_demo -- <video folder>
//! ```
use std::path::Path;

use studio_agent::prompt::{self, LookSheet, LookTile};
use studio_render::critic::{Critic, SHEET_COLS, SHEET_ROWS, contact_sheets, look_frames};
use studio_render::preview::OpenScene;
use studio_render::{Output, slides};

fn main() -> anyhow::Result<()> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: review_demo <video folder>"));
    let ffmpeg = studio_render::ffmpeg_path().expect("ffmpeg");
    let out = dir.join(".cache").join("review");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out)?;
    let theme_json = studio_render::style::json(0).to_string();
    let rel = |p: &Path| p.strip_prefix(&dir).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let mut sheets: Vec<LookSheet> = Vec::new();
    let mut number = 1;
    for (i, file) in studio_render::sample::scene_files(&dir).into_iter().enumerate() {
        let req = OpenScene { path: file.clone(), root: dir.clone(), theme_json: theme_json.clone(), scene: None, globals: vec![], narration: None, narration_lines: vec![], narration_cues: vec![], critic: Some(Critic::default()) };
        let stills = look_frames(&req, Output::new(1280, 720, 30.0))?;
        let refs: Vec<&studio_render::Frame> = stills.iter().map(|(_, f)| f).collect();
        let per = SHEET_COLS * SHEET_ROWS;
        for (s, sheet) in contact_sheets(&refs, number).iter().enumerate() {
            let img = out.join(format!("{:02}-{}.jpg", i + 1, s + 1));
            slides::frame_to_jpeg(&ffmpeg, sheet, &img)?;
            let tiles = stills
                .iter()
                .skip(s * per)
                .take(per)
                .enumerate()
                .map(|(k, (m, _))| LookTile { n: number + s * per + k, chapter: i + 1, file: rel(&file), at: m.time, line: m.line, said: m.said.clone() })
                .collect();
            sheets.push(LookSheet { image: rel(&img), tiles });
        }
        number += stills.len();
    }
    std::fs::write(dir.join("review-prompt.txt"), prompt::look(&sheets))?;
    std::fs::write(dir.join("review-system.md"), prompt::build_system(&prompt::api_digest(studio_render::TYPES)))?;
    println!("{} sheets, {} stills", sheets.len(), number - 1);
    Ok(())
}
