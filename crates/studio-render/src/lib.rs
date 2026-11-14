//! Manim Studio's engine layer, built on [manim-rust](https://github.com/ItsTatsuya/manim-rust).
//!
//! - [`preview`]: live stage preview. A scene script runs once without rendering
//!   (`dry_run` + `record`), then any time can be shown with `frame_at`. Each preview
//!   engine owns one [`manim::script::ScriptHost`] on its own thread, because the
//!   script host keeps its state in thread-locals.
//! - [`queue`]: final renders. Every scene renders in a separate engine process
//!   ([`worker`]) that reports with `--progress json`, so a crash or a runaway script
//!   never takes the app down and cancelling is a kill.
//! - [`cache`]: scene renders are keyed by a hash of the script, its imports, the house
//!   style and the output settings, so an edit re-renders one scene, not the video.
//! - [`export`]: scene videos are joined into the lesson video with ffmpeg.
//!
//! Nothing here depends on the UI toolkit: frames come out as BGRA bytes ready to
//! upload as a texture.

mod fetch;
pub mod tools;
pub mod cache;
pub mod events;
pub mod export;
pub mod fonts;
pub mod kokoro;
pub mod preview;
pub mod queue;
pub mod sample;
pub mod showcase;
pub mod style;
pub mod voice;
pub mod worker;

pub use manim;
pub use manim::scene::TimelineInfo;

/// The engine revision this build is pinned to (part of every cache key).
pub const ENGINE_REV: &str = "manim-rust@3deae09";

/// TypeScript declarations of the scene API (for editors and the assistant's context).
pub const TYPES: &str = manim::script::TYPES;

/// One rasterized frame, BGRA8 (the GPU texture format GPUI expects), row-major.
#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Frame {
    /// Take an RGBA8 frame from the engine and reorder it to BGRA in place.
    pub fn from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Self {
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        Frame { width, height, bgra: rgba }
    }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{})", self.width, self.height)
    }
}

/// Output size and frame rate of a render.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

impl Output {
    pub const fn new(width: u32, height: u32, fps: f64) -> Self {
        Output { width, height, fps }
    }

    /// "1080p · 30" style label.
    pub fn label(&self) -> String {
        let short = self.width.min(self.height);
        let name = if short >= 2160 { "4K".to_string() } else { format!("{short}p") };
        let vertical = if self.height > self.width { " vertical" } else { "" };
        format!("{name}{vertical} · {}", self.fps.round() as u32)
    }
}

/// How the final render rasterizes and encodes (maps to `--renderer` / `--encoder`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Auto,
    Gpu,
    Cpu,
}

impl Backend {
    pub fn flag(self) -> &'static str {
        match self {
            Backend::Auto => "auto",
            Backend::Gpu => "gpu",
            Backend::Cpu => "cpu",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoder {
    /// First working hardware encoder at ≥1080p30, else x264.
    #[default]
    Auto,
    X264,
}

impl Encoder {
    pub fn flag(self) -> &'static str {
        match self {
            Encoder::Auto => "auto",
            Encoder::X264 => "x264",
        }
    }
}

/// Whether ffmpeg can be found (MP4 output and joining scenes need it).
/// ffmpeg: the copy Studio downloaded, else one on PATH or from winget.
pub fn ffmpeg_path() -> Option<std::path::PathBuf> {
    tools::ffmpeg().or_else(manim::output::find_ffmpeg)
}

/// A path as text without the Windows verbatim prefix (`\\?\`), for ffmpeg and messages.
pub fn plain_path(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => rest.to_string(),
        _ => s.into_owned(),
    }
}

/// First line of an engine error, for compact display (the rest is the stack and source excerpt).
pub fn short_error(message: &str) -> String {
    let first = message.lines().find(|l| !l.trim().is_empty()).unwrap_or(message).trim();
    first.strip_prefix("Error: ").unwrap_or(first).to_string()
}

/// The TypeScript line an engine error points at (`file.ts:LINE`), if any.
pub fn error_line(message: &str) -> Option<u32> {
    let i = message.find(".ts:")?;
    message[i + 4..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}
