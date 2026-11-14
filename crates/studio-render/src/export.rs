//! Join scene videos into the lesson video.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// Spawn without flashing a console window on Windows.
pub(crate) fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn run(ffmpeg: &Path, args: &[&std::ffi::OsStr]) -> Result<()> {
    let out = quiet(Command::new(ffmpeg).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()))
        .output()
        .with_context(|| format!("running {}", ffmpeg.display()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg failed");
        bail!("{last}");
    }
    Ok(())
}

/// Concatenate `parts` (same size and frame rate) into `out`. Streams are copied when
/// they line up; otherwise the parts are re-encoded once with x264.
pub fn concat(ffmpeg: &Path, parts: &[PathBuf], out: &Path) -> Result<()> {
    if parts.is_empty() {
        bail!("nothing to export");
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if parts.len() == 1 {
        std::fs::copy(&parts[0], out).with_context(|| format!("writing {}", out.display()))?;
        return Ok(());
    }
    let list = out.with_extension("parts.txt");
    let mut body = String::new();
    for p in parts {
        let abs = p.canonicalize().unwrap_or_else(|_| p.clone());
        let s = crate::plain_path(&abs).replace('\'', "'\\''");
        body.push_str(&format!("file '{s}'\n"));
    }
    std::fs::write(&list, body)?;
    let tmp = out.with_extension("partial.mp4");
    let base: Vec<&std::ffi::OsStr> = vec![
        "-y".as_ref(),
        "-hide_banner".as_ref(),
        "-loglevel".as_ref(),
        "error".as_ref(),
        "-f".as_ref(),
        "concat".as_ref(),
        "-safe".as_ref(),
        "0".as_ref(),
        "-i".as_ref(),
        list.as_os_str(),
    ];
    let mut copy = base.clone();
    copy.extend::<[&std::ffi::OsStr; 5]>(["-c".as_ref(), "copy".as_ref(), "-movflags".as_ref(), "+faststart".as_ref(), tmp.as_os_str()]);
    let result = run(ffmpeg, &copy).or_else(|_| {
        let mut enc = base.clone();
        enc.extend::<[&std::ffi::OsStr; 11]>([
            "-c:v".as_ref(),
            "libx264".as_ref(),
            "-preset".as_ref(),
            "veryfast".as_ref(),
            "-pix_fmt".as_ref(),
            "yuv420p".as_ref(),
            "-c:a".as_ref(),
            "aac".as_ref(),
            "-movflags".as_ref(),
            "+faststart".as_ref(),
            tmp.as_os_str(),
        ]);
        run(ffmpeg, &enc)
    });
    let _ = std::fs::remove_file(&list);
    result?;
    std::fs::rename(&tmp, out).with_context(|| format!("writing {}", out.display()))?;
    Ok(())
}

/// Where along an axis: start (left/top), centre, or end (right/bottom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

impl Align {
    /// 0, 1, 2 → Start, Center, End.
    pub fn from_index(i: usize) -> Self {
        match i {
            0 => Align::Start,
            1 => Align::Center,
            _ => Align::End,
        }
    }
}

/// What the watermark shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    Text(String),
    /// A PNG, JPEG or WebP (transparency kept).
    Image(PathBuf),
}

/// A mark drawn on the exported video.
#[derive(Clone, Debug, PartialEq)]
pub struct Watermark {
    pub mark: Mark,
    /// Horizontal and vertical placement.
    pub at: (Align, Align),
    /// Image: fraction of the video width. Text: font size as a fraction of the height.
    pub size: f32,
    /// 0..=1.
    pub opacity: f32,
    /// Gap to the edges, as a fraction of the shorter side.
    pub margin: f32,
    /// `RRGGBB` text colour; `None` uses the house style's text colour.
    pub color: Option<String>,
    /// A soft shadow (style background colour) behind text.
    pub shadow: bool,
}

/// The watermark's top-left corner and box size for a `width`×`height` video, given the
/// mark's own size. Shared by the export and the Settings preview so they agree.
pub fn place(at: (Align, Align), margin: f32, width: f32, height: f32, mark_w: f32, mark_h: f32) -> (f32, f32) {
    let m = width.min(height) * margin;
    let axis = |a: Align, full: f32, size: f32| match a {
        Align::Start => m,
        Align::Center => (full - size) / 2.0,
        Align::End => full - size - m,
    };
    (axis(at.0, width, mark_w), axis(at.1, height, mark_h))
}

/// The last pass over the joined video: burned-in captions and/or a watermark.
pub struct Finish<'a> {
    pub captions: Option<&'a Path>,
    pub watermark: Option<&'a Watermark>,
    /// TTF for text watermarks (the bundled Inter).
    pub font: Option<&'a Path>,
    /// `RRGGBB` for watermark text and its shadow (from the house style).
    pub text_rgb: &'a str,
    pub shadow_rgb: &'a str,
    pub width: u32,
    pub height: u32,
}

/// `#RRGGBB` → `RRGGBB` for ffmpeg colours, with a fallback.
pub fn rgb(hex: Option<&str>, fallback: &str) -> String {
    match hex.map(|h| h.trim_start_matches('#')) {
        Some(h) if h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()) => h.to_string(),
        _ => fallback.to_string(),
    }
}

/// Re-encode `video` with captions and/or a watermark drawn on it.
pub fn finish(ffmpeg: &Path, video: &Path, f: &Finish, out: &Path) -> Result<()> {
    // Filter graphs need escaped paths; copy everything into a scratch folder and run
    // ffmpeg there so every name is plain.
    let work = out.with_extension("finish");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let result = (|| -> Result<()> {
        let mut graph: Vec<String> = Vec::new();
        let mut cur = "0:v".to_string();
        let mut inputs: Vec<PathBuf> = vec![video.to_path_buf()];
        if let Some(srt) = f.captions {
            std::fs::copy(srt, work.join("captions.srt"))?;
            graph.push(format!(
                "[{cur}]subtitles='captions.srt':force_style='FontName=Inter,FontSize=18,BorderStyle=3,Outline=1,Shadow=0,BackColour=&H80000000,MarginV=28'[c]"
            ));
            cur = "c".into();
        }
        if let Some(w) = f.watermark {
            let op = w.opacity.clamp(0.05, 1.0);
            let m = ((f.width.min(f.height) as f32) * w.margin.clamp(0.0, 0.3)).round() as u32;
            // ffmpeg expressions matching `place`: `big`/`small` are the frame and mark sizes.
            let pos = |a: Align, big: &str, small: &str| match a {
                Align::Start => m.to_string(),
                Align::Center => format!("({big}-{small})/2"),
                Align::End => format!("{big}-{small}-{m}"),
            };
            match &w.mark {
                Mark::Image(path) => {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
                    let name = format!("mark.{ext}");
                    std::fs::copy(path, work.join(&name)).with_context(|| format!("reading {}", path.display()))?;
                    inputs.push(PathBuf::from(name));
                    let px = ((f.width as f32 * w.size).round() as u32).max(8);
                    let (x, y) = (pos(w.at.0, "W", "w"), pos(w.at.1, "H", "h"));
                    graph.push(format!("[1:v]format=rgba,scale={px}:-1,colorchannelmixer=aa={op:.2}[wm]"));
                    graph.push(format!("[{cur}][wm]overlay=x={x}:y={y}:format=auto[v]"));
                }
                Mark::Text(text) => {
                    std::fs::write(work.join("mark.txt"), text.trim())?;
                    let font = match f.font {
                        Some(p) => {
                            std::fs::copy(p, work.join("font.ttf"))?;
                            "fontfile='font.ttf':".to_string()
                        }
                        None => String::new(),
                    };
                    let size = ((f.height as f32 * w.size).round() as u32).max(8);
                    let (x, y) = (pos(w.at.0, "w", "tw"), pos(w.at.1, "h", "th"));
                    let color = w.color.as_deref().unwrap_or(f.text_rgb);
                    let shadow = if w.shadow {
                        let d = (size / 18).max(1);
                        format!(":shadowcolor=0x{}@{:.2}:shadowx={d}:shadowy={d}", f.shadow_rgb, op * 0.6)
                    } else {
                        String::new()
                    };
                    graph.push(format!(
                        "[{cur}]drawtext={font}textfile='mark.txt':expansion=none:fontsize={size}:fontcolor=0x{color}@{op:.2}{shadow}:x={x}:y={y}[v]"
                    ));
                }
            }
            cur = "v".into();
        }
        if cur == "0:v" {
            bail!("nothing to add");
        }
        if cur != "v" {
            // Captions only: rename the last label.
            let last = graph.pop().unwrap_or_default();
            graph.push(format!("{}[v]", last.trim_end_matches(&format!("[{cur}]"))));
        }
        let tmp = out.with_extension("partial.mp4");
        let mut cmd = Command::new(ffmpeg);
        cmd.current_dir(&work).args(["-y", "-hide_banner", "-loglevel", "error"]);
        for i in &inputs {
            cmd.arg("-i").arg(i);
        }
        cmd.args(["-filter_complex", &graph.join(";"), "-map", "[v]", "-map", "0:a?"])
            .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "copy", "-movflags", "+faststart"])
            .arg(&tmp)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let st = quiet(&mut cmd).output()?;
        if !st.status.success() {
            let err = String::from_utf8_lossy(&st.stderr);
            bail!("{}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg failed"));
        }
        std::fs::rename(&tmp, out)?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Longest cross-dissolve between scenes; shorter when a scene is very short.
pub const DISSOLVE: f64 = 0.5;

/// The dissolve length used for scenes of these lengths (0 when there is nothing to join).
pub fn dissolve_for(durations: &[f64]) -> f64 {
    if durations.len() < 2 {
        return 0.0;
    }
    let shortest = durations.iter().cloned().fold(f64::INFINITY, f64::min);
    DISSOLVE.min(shortest / 3.0).max(0.0)
}

/// Join scenes into one continuous video: the picture cross-dissolves from each scene
/// into the next, and each scene's audio starts where its picture does (the narration
/// tail covers the overlap, so voices never collide). Falls back to a plain join if the
/// filter graph fails.
pub fn join_dissolve(ffmpeg: &Path, parts: &[PathBuf], durations: &[f64], audio: bool, out: &Path) -> Result<()> {
    let fade = dissolve_for(durations);
    if parts.len() < 2 || durations.len() != parts.len() || fade < 0.05 {
        return concat(ffmpeg, parts, out);
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut graph: Vec<String> = Vec::new();
    let mut prev = "0:v".to_string();
    let mut start = 0.0;
    let mut starts = vec![0.0];
    for (i, d) in durations.iter().enumerate().take(parts.len() - 1) {
        start += d - fade;
        starts.push(start);
        let label = if i + 2 == parts.len() { "v".to_string() } else { format!("x{}", i + 1) };
        graph.push(format!("[{prev}][{}:v]xfade=transition=fade:duration={fade:.3}:offset={start:.3}[{label}]", i + 1));
        prev = label;
    }
    if audio {
        let mut mix = String::new();
        for (i, s) in starts.iter().enumerate() {
            let ms = (s * 1000.0).round() as u64;
            graph.push(format!("[{i}:a]aresample=48000,adelay=delays={ms}:all=1[a{i}]"));
            mix.push_str(&format!("[a{i}]"));
        }
        graph.push(format!("{mix}amix=inputs={}:normalize=0:dropout_transition=0[a]", parts.len()));
    }
    let tmp = out.with_extension("partial.mp4");
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
    for p in parts {
        cmd.arg("-i").arg(p);
    }
    cmd.args(["-filter_complex", &graph.join(";"), "-map", "[v]"]);
    if audio {
        cmd.args(["-map", "[a]", "-c:a", "aac", "-b:a", "192k"]);
    }
    cmd.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let st = quiet(&mut cmd).output()?;
    if !st.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return concat(ffmpeg, parts, out);
    }
    std::fs::rename(&tmp, out).with_context(|| format!("writing {}", out.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs ffmpeg: three 2 s clips dissolve into one 2 × 3 − 2 × 0.5 = 5 s video with audio.
    #[test]
    fn dissolves_between_scenes() {
        let Some(ff) = crate::ffmpeg_path() else { return };
        let dir = std::env::temp_dir().join(format!("studio-join-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut parts = Vec::new();
        for (i, hz) in [220, 330, 440].iter().enumerate() {
            let p = dir.join(format!("p{i}.mp4"));
            let st = quiet(Command::new(&ff).args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=15:duration=2", "-f", "lavfi", "-i"]).arg(format!("sine=frequency={hz}:duration=2")).args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"]).arg(&p))
                .status()
                .unwrap();
            assert!(st.success());
            parts.push(p);
        }
        let out = dir.join("joined.mp4");
        join_dissolve(&ff, &parts, &[2.0, 2.0, 2.0], true, &out).unwrap();
        let probe = ff.with_file_name(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" });
        if probe.is_file() {
            let o = Command::new(&probe).args(["-v", "error", "-show_entries", "stream=codec_type,duration", "-of", "csv=p=0"]).arg(&out).output().unwrap();
            let text = String::from_utf8_lossy(&o.stdout);
            assert!(text.contains("video") && text.contains("audio"), "{text}");
            let v: f64 = text.lines().find(|l| l.starts_with("video")).and_then(|l| l.split(',').nth(1)).and_then(|d| d.parse().ok()).unwrap();
            assert!((v - 5.0).abs() < 0.15, "{text}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Needs ffmpeg (skipped otherwise): text, image and captions + text on a test clip.
    #[test]
    fn draws_watermarks() {
        let Some(ff) = crate::ffmpeg_path() else { return };
        let dir = std::env::temp_dir().join(format!("studio-wm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mp4");
        let st = quiet(Command::new(&ff).args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=15:duration=1", "-pix_fmt", "yuv420p"]).arg(&clip))
            .status()
            .unwrap();
        assert!(st.success());
        let png = dir.join("logo.png");
        let rgba: Vec<u8> = (0..64 * 32).flat_map(|i| [255, (i % 255) as u8, 0, if i % 7 == 0 { 0 } else { 200 }]).collect();
        manim::output::save_png(&png, 64, 32, &rgba).unwrap();
        let srt = dir.join("c.srt");
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,000\nHello there\n").unwrap();
        let font = [r"C:\Windows\Fonts\arial.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file());
        let marks = [
            (None, Mark::Text("Ms. Rao · Grade 8 it's 100%".into()), (Align::End, Align::End)),
            (None, Mark::Image(png.clone()), (Align::Start, Align::Center)),
            (Some(srt.as_path()), Mark::Text("School".into()), (Align::Center, Align::Start)),
        ];
        for (k, (captions, mark, at)) in marks.into_iter().enumerate() {
            if matches!(mark, Mark::Text(_)) && font.is_none() {
                continue;
            }
            let wm = Watermark { mark, at, size: 0.08, opacity: 0.6, margin: 0.035, color: (k == 2).then(|| "FF0000".into()), shadow: k != 2 };
            let out = dir.join(format!("out{k}.mp4"));
            let f = Finish { captions, watermark: Some(&wm), font: font.as_deref(), text_rgb: "FFFFFF", shadow_rgb: "000000", width: 640, height: 360 };
            finish(&ff, &clip, &f, &out).unwrap_or_else(|e| panic!("mark {k}: {e:#}"));
            assert!(std::fs::metadata(&out).unwrap().len() > 1000);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
