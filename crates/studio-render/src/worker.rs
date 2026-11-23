//! The engine worker process: renders one scene script and reports progress as JSON
//! lines on stderr (manim-rust's `--progress json` protocol).
//!
//! ```text
//! manim-studio render-worker --script scenes/03-unroll.ts --root <video> [--timeout 600]
//!     [--narration track.wav | --narration-part s1.wav --narration-part s2.wav ... --narration-starts out.starts
//!      --narration-cues '[{"id":"c20","line":1,"at":1.2}]']
//!     [--sfx-marks out.sfx]
//!     [manim flags: -r 1920x1080 --fps 30 --format mp4 -o out.mp4 --theme style.json --fonts DIR ...]
//! ```
//!
//! Running each render in its own process means a crash, a runaway script or a
//! misbehaving GPU driver can't take the app down, and cancelling is a kill.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use manim::config::Config;
use manim::script::{HostOptions, ScriptHost};

/// First argument that selects worker mode.
pub const ARG: &str = "render-worker";

/// Run the worker with the arguments after [`ARG`]. Returns the process exit code.
pub fn main(args: impl IntoIterator<Item = String>) -> i32 {
    match run(args) {
        Ok(()) => 0,
        Err(e) => {
            // `render` already reported script errors as an `error` event; this covers
            // bad arguments and setup failures.
            let msg = serde_json::json!({ "event": "error", "scene": "", "message": format!("{e:#}") });
            eprintln!("{msg}");
            1
        }
    }
}

fn run(args: impl IntoIterator<Item = String>) -> Result<()> {
    let mut script: Option<PathBuf> = None;
    let mut root: Option<PathBuf> = None;
    let mut timeout = Duration::from_secs(600);
    let mut narration: Option<PathBuf> = None;
    let mut parts: Vec<PathBuf> = Vec::new();
    let mut starts_file: Option<PathBuf> = None;
    let mut cues: Vec<crate::cues::CueTime> = Vec::new();
    let mut sfx_file: Option<PathBuf> = None;
    let mut silence = false;
    let mut rest = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--script" => script = Some(it.next().context("--script needs a path")?.into()),
            "--root" => root = Some(it.next().context("--root needs a folder")?.into()),
            "--narration" => narration = Some(it.next().context("--narration needs a WAV file")?.into()),
            "--narration-part" => parts.push(it.next().context("--narration-part needs a WAV file")?.into()),
            "--narration-starts" => starts_file = Some(it.next().context("--narration-starts needs a path")?.into()),
            "--narration-cues" => {
                cues = serde_json::from_str(&it.next().context("--narration-cues needs JSON")?).context("reading --narration-cues")?;
            }
            "--sfx-marks" => sfx_file = Some(it.next().context("--sfx-marks needs a path")?.into()),
            "--silence" => silence = true,
            "--timeout" => {
                timeout = Duration::from_secs_f64(it.next().context("--timeout needs seconds")?.parse()?);
            }
            _ => rest.push(a),
        }
    }
    let Some(script) = script else { bail!("--script is required") };
    let config = Config::parse(rest)?;
    let options = HostOptions { sandbox: root, timeout: Some(timeout), memory_limit: Some(1024 * 1024 * 1024) };

    // Narration: one track (`--narration`), or sentences (`--narration-part`, in order)
    // that the scene places with `this.narrate(n)`.
    let lines: Vec<f64> = parts.iter().map(|p| crate::voice::wav_seconds(p)).collect::<Result<_>>()?;
    let seconds = match (&narration, lines.is_empty()) {
        (_, false) => {
            let st = crate::voice::sequential_starts(&lines);
            Some(crate::preview::narration_end(&st, &lines))
        }
        (Some(wav), true) => Some(crate::voice::wav_seconds(wav)?),
        (None, true) => None,
    };
    let scene_of = |host: &ScriptHost| -> Result<String> {
        Ok(match &config.scene_filter {
            Some(s) => s.clone(),
            None => host.scenes().into_iter().next().context("no scenes")?,
        })
    };

    // A dry run first for narrated scenes: does the scene place its sentences itself, and
    // if not, how should its pauses shrink to end with the voice (the same plan the
    // preview uses, so preview and render agree)?
    let mut plan = None;
    if let Some(n) = seconds {
        let dry = ScriptHost::load(&script, options.clone())?;
        let scene = scene_of(&dry)?;
        dry.set_global_json("NARRATION", &crate::preview::narration_json(n, &lines, &cues))?;
        crate::preview::install_hooks(&dry, None)?;
        let mut c = config.clone();
        c.dry_run = true;
        c.record = true;
        c.progress = manim::events::ProgressMode::None;
        dry.run(&scene, &c)?;
        if let Some(t) = dry.timeline()
            && !crate::preview::uses_narrate(&t)
        {
            plan = crate::preview::timing_plan(&t, n);
        }
        drop(dry);
    }

    let host = ScriptHost::load(&script, options)?;
    let scene = scene_of(&host)?;
    if let Some(n) = seconds {
        host.set_global_json("NARRATION", &crate::preview::narration_json(n, &lines, &cues))?;
    }
    crate::preview::install_hooks(&host, plan.as_ref())?;
    let events = manim::events::Events::new(config.progress);
    // The sentence track is mixed into a temporary WAV that must outlive `finish`.
    let mut track: Option<PathBuf> = None;
    let result = host.run(&scene, &config).and_then(|_| {
        let timeline = host.timeline();
        // Where the lesson kit asked for soft sounds; the export places them if wanted.
        if let (Some(f), Some(t)) = (&sfx_file, &timeline) {
            let _ = std::fs::write(f, serde_json::to_string(&crate::mix::hits(&t.markers))?);
        }
        let placed = !lines.is_empty() && timeline.as_ref().is_some_and(crate::preview::uses_narrate);
        if let Some(n) = seconds {
            let starts = match (&timeline, placed) {
                (Some(t), true) => crate::preview::narration_starts(t, &lines),
                _ => crate::voice::sequential_starts(&lines),
            };
            let until = if lines.is_empty() { n } else { crate::preview::narration_end(&starts, &lines) };
            crate::preview::pad_for_narration(&host, until);
            if !lines.is_empty() {
                let dir = std::env::temp_dir().join(format!("studio-narration-{}", std::process::id()));
                let line = crate::voice::compose(&parts, &starts, &dir)?;
                if let Some(f) = &starts_file {
                    let _ = std::fs::write(f, serde_json::to_string(&starts)?);
                }
                track = Some(line.wav);
            }
        }
        host.with_scene(|s| match (&track, &narration) {
            (Some(wav), _) | (None, Some(wav)) => s.add_sound_at(wav, 0.0, 1.0),
            (None, None) if silence => {
                let rate = 24_000u32;
                let n = (s.duration().max(0.1) * rate as f64) as usize;
                s.add_samples(std::sync::Arc::from(vec![0.0f32; n]), rate, 1, 0.0, 1.0);
            }
            (None, None) => {}
        });
        host.finish()
    });
    if let Some(t) = &track
        && let Some(dir) = t.parent()
    {
        let _ = std::fs::remove_dir_all(dir);
    }
    if let Err(e) = &result {
        events.emit("error", serde_json::json!({ "scene": scene, "message": format!("{e:#}") }), || None);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs ffmpeg (skipped otherwise): a worker run saves where the lesson kit asked for
    /// soft sounds, beside the scene, for the export to read.
    #[test]
    fn saves_where_sounds_belong() {
        if crate::ffmpeg_path().is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("studio-worker-sfx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        crate::kit::install(&dir).unwrap();
        let script = dir.join("scenes/01-a.ts");
        std::fs::write(
            &script,
            "import { Scene, Text } from \"manim\";\n\
             import { work, figure } from \"../lib/layout\";\n\
             import { lesson } from \"../lib/lesson\";\n\
             export default class A extends Scene { construct() {\n\
               const k = lesson(this);\n\
               const plot = k.Plot({ xRange: [0, 40, 10], yRange: [0, 40, 10] });\n\
               this.play(plot.show(), { runTime: 0.4 });\n\
               this.play(plot.dot([10, 20], k.c.primary), { runTime: 0.3 });\n\
               this.play(k.AnswerBox(\"r = 10\").enter(), { runTime: 0.3 });\n\
             } }",
        )
        .unwrap();
        let (out, sfx) = (dir.join("scene.mp4"), dir.join("scene.sfx"));
        let args = ["--script", script.to_str().unwrap(), "--root", dir.to_str().unwrap(), "-r", "160x90", "--fps", "10", "--format", "mp4", "--renderer", "cpu", "--encoder", "x264", "-o", out.to_str().unwrap(), "--sfx-marks", sfx.to_str().unwrap()];
        assert_eq!(main(args.map(String::from)), 0);
        let hits = crate::mix::load_hits(&sfx);
        assert_eq!(hits.iter().map(|h| h.kind).collect::<Vec<_>>(), [crate::mix::Sfx::Pop, crate::mix::Sfx::Answer], "{hits:?}");
        assert!((hits[0].at - 0.4).abs() < 0.15 && (hits[1].at - 0.7).abs() < 0.15, "{hits:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
