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
pub mod cues;
pub mod equations;
pub mod kit;
pub mod events;
pub mod export;
pub mod fonts;
pub mod kokoro;
pub mod preview;
pub mod queue;
pub mod sample;
pub mod showcase;
pub mod slides;
pub mod style;
pub mod voice;
pub mod worker;

pub use manim;
pub use manim::scene::TimelineInfo;

/// The engine revision this build is pinned to (part of every cache key). Derived by
/// `build.rs` from the `manim` entry in the workspace `Cargo.lock`, so it can't go stale.
pub const ENGINE_REV: &str = env!("STUDIO_ENGINE_REV");

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

// ---------------------------------------------------------------------- engine guard

thread_local! {
    static ENGINE_GUARD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// True while the calling thread is inside one of Studio's guarded engine calls (scene
/// load, frame, thumbnail, clip and check). The engine runs untrusted scripts and can
/// panic; those panics are caught and reported as ordinary errors, so an app-wide panic
/// hook should treat a panic raised while this is true as recovered rather than a crash.
/// The hook runs on the panicking thread before unwinding, so a thread-local is enough.
pub fn in_engine_guard() -> bool {
    ENGINE_GUARD.with(|g| g.get() > 0)
}

/// The error a caught engine panic becomes. After one, the `ScriptHost` that panicked
/// must be dropped and rebuilt (it keeps its state in thread-locals).
#[derive(Debug)]
pub struct EnginePanic(pub String);

impl std::fmt::Display for EnginePanic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the engine hit an internal error ({})", self.0)
    }
}

impl std::error::Error for EnginePanic {}

/// Whether `e` came from a caught engine panic.
pub fn is_engine_panic(e: &anyhow::Error) -> bool {
    e.downcast_ref::<EnginePanic>().is_some()
}

struct GuardDepth;

impl GuardDepth {
    fn enter() -> Self {
        ENGINE_GUARD.with(|g| g.set(g.get() + 1));
        GuardDepth
    }
}

impl Drop for GuardDepth {
    fn drop(&mut self) {
        ENGINE_GUARD.with(|g| g.set(g.get().saturating_sub(1)));
    }
}

/// Run an engine call, turning a panic into an [`EnginePanic`] error and marking the
/// thread as [`in_engine_guard`] meanwhile.
pub(crate) fn guard<T>(f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    let _depth = GuardDepth::enter();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            Err(EnginePanic(msg.lines().next().unwrap_or("panic").chars().take(160).collect()).into())
        }
    }
}

/// The error a cancelled export step or download returns (its `cancel` flag or callback
/// said stop). Not a failure: callers show "cancelled", not an error.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Whether `e` is [`Cancelled`].
pub fn is_cancelled(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Cancelled>().is_some()
}

/// Lock a mutex, ignoring poisoning. With `panic = "unwind"` a panic on another thread
/// can poison long-lived shared state; the data these locks guard stays valid, so the
/// next user simply carries on.
pub(crate) fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_catches_panics_and_sets_the_flag() {
        assert!(!in_engine_guard());
        let r: anyhow::Result<()> = guard(|| {
            assert!(in_engine_guard());
            panic!("boom");
        });
        let e = r.unwrap_err();
        assert!(is_engine_panic(&e) && e.to_string().contains("boom"), "{e}");
        assert!(!in_engine_guard());
        assert!(guard(|| guard(|| Ok(in_engine_guard()))).unwrap());
        assert!(!in_engine_guard());
    }
}

