//! Render a video folder headlessly, the way Studio does: narration spoken sentence by
//! sentence (Kokoro when downloaded, else the system voice), scenes through the render
//! queue and worker, dissolve join, captions.
//!
//! ```sh
//! cargo build -p studio-app --bin studio-engine
//! cargo run -p studio-render --release --example render_video -- <video folder> [out.mp4]
//! ```
//! The narration comes from the folder's `video.json` storyboard (`scenes[i].say`). An
//! empty or missing folder gets the bundled sample lesson.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use studio_render::queue::{Captions, Narration, QueueEvent, RenderQueue, SceneSpec, VideoRender, WorkerCommand};
use studio_render::{Backend, Encoder, Output, sample, style, voice};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.first().context("usage: render_video <video folder> [out.mp4]")?);
    let out = args.get(1).map(PathBuf::from).unwrap_or_else(|| dir.join("exports").join("headless.mp4"));
    if let Some(data) = std::env::var_os("APPDATA") {
        voice::set_kokoro_dir(Path::new(&data).join("ManimStudio/Manim Studio/data/voices/kokoro"));
    }
    // An empty folder gets the bundled sample lesson.
    if !dir.join("video.json").is_file() {
        sample::install(&dir)?;
    }
    let video: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("video.json"))?)?;
    let says: Vec<String> = video["storyboard"]["scenes"]
        .as_array()
        .map(|a| a.iter().map(|b| b["say"].as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    let spec = voice::VoiceSpec { voice: None, rate: 1.0 };
    let cache = voice::cache_dir(&dir);
    let mut scenes = Vec::new();
    for (i, path) in sample::scene_files(&dir).into_iter().enumerate() {
        let say = says.get(i).cloned().unwrap_or_default();
        let narration = if say.trim().is_empty() {
            None
        } else {
            let parts: Vec<(PathBuf, String, f64)> = voice::sentences(&say)
                .into_iter()
                .map(|s| voice::line(&s, &spec, &cache).map(|l| (l.wav, s, l.seconds)))
                .collect::<Result<_>>()?;
            let lines: Vec<f64> = parts.iter().map(|p| p.2).collect();
            let wavs: Vec<PathBuf> = parts.iter().map(|p| p.0.clone()).collect();
            let track = voice::compose(&wavs, &voice::sequential_starts(&lines), &cache)?;
            println!("scene {}: {} sentences, {:.1} s of speech", i + 1, parts.len(), lines.iter().sum::<f64>());
            Some(Narration { wav: track.wav, seconds: track.seconds, text: say, parts: parts.into_iter().map(|p| (p.0, p.1)).collect() })
        };
        scenes.push(SceneSpec { path, expected: None, narration });
    }
    let exe = std::env::current_exe()?;
    let engine = exe.parent().and_then(|p| p.parent()).map(|p| p.join(if cfg!(windows) { "studio-engine.exe" } else { "studio-engine" }));
    let worker = match engine.filter(|e| e.is_file()) {
        Some(e) => WorkerCommand { exe: e, args: vec![] },
        None => anyhow::bail!("build the worker first: cargo build -p studio-app --release --bin studio-engine"),
    };
    let queue = RenderQueue::spawn(worker);
    queue.push(VideoRender {
        id: 1,
        root: dir.clone(),
        scenes,
        style_json: style::json(0).to_string(),
        output: Output::new(1280, 720, 30.0),
        backend: Backend::Auto,
        encoder: Encoder::Auto,
        export: out.clone(),
        fonts_dir: None,
        captions: Captions::Srt,
        watermark: None,
    });
    while let Ok(ev) = queue.events().recv_blocking() {
        match ev {
            QueueEvent::Scene { index, of, cached, .. } => println!("scene {}/{of}{}", index + 1, if cached { " (cached)" } else { "" }),
            QueueEvent::Done { path, duration, elapsed, .. } => {
                println!("done: {} ({duration:.1} s video in {:.0} s)", path.display(), elapsed.as_secs_f64());
                return Ok(());
            }
            QueueEvent::Failed { error, scene, .. } => anyhow::bail!("scene {scene:?}: {error}"),
            QueueEvent::Cancelled { .. } => anyhow::bail!("cancelled"),
            _ => {}
        }
    }
    Ok(())
}
