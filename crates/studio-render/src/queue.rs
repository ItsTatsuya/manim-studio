//! The render queue: one video at a time, one engine process per scene, cached scenes
//! skipped, then the scenes are joined into the lesson video.

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::events::EngineEvent;
use crate::{Backend, Encoder, Output, cache, export};

/// How to start an engine worker process (the app passes its own executable plus
/// [`crate::worker::ARG`]).
#[derive(Clone, Debug)]
pub struct WorkerCommand {
    pub exe: PathBuf,
    pub args: Vec<String>,
}

impl WorkerCommand {
    /// This executable in worker mode.
    pub fn current() -> std::io::Result<Self> {
        Ok(WorkerCommand { exe: std::env::current_exe()?, args: vec![crate::worker::ARG.to_string()] })
    }
}

#[derive(Clone, Debug)]
pub struct SceneSpec {
    pub path: PathBuf,
    /// Duration from a preview run, for overall progress (equal weights when unknown).
    pub expected: Option<f64>,
    /// The scene's narration: its WAV and the text it speaks (for captions).
    pub narration: Option<Narration>,
}

#[derive(Clone, Debug)]
pub struct Narration {
    /// The whole track (sentences laid out for the preview).
    pub wav: PathBuf,
    pub seconds: f64,
    pub text: String,
    /// Each sentence's WAV and text. The engine places them where the scene calls
    /// `narrate(n)` and writes their start times next to the scene video.
    pub parts: Vec<(PathBuf, String)>,
}

/// What to do with captions at export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Captions {
    Off,
    /// A `.srt` file next to the video.
    #[default]
    Srt,
    /// Drawn on the video, plus the `.srt`.
    Burned,
}

#[derive(Clone, Debug)]
pub struct VideoRender {
    pub id: u64,
    /// The video folder (sandbox for scripts and assets, home of `.cache/`).
    pub root: PathBuf,
    pub scenes: Vec<SceneSpec>,
    pub style_json: String,
    pub output: Output,
    pub backend: Backend,
    pub encoder: Encoder,
    /// Where the finished lesson video goes.
    pub export: PathBuf,
    /// Extra fonts for the engine (`--fonts`).
    pub fonts_dir: Option<PathBuf>,
    pub captions: Captions,
    /// Drawn on the exported video (not on the cached scenes).
    pub watermark: Option<export::Watermark>,
}

#[derive(Clone, Debug)]
pub enum QueueEvent {
    Started { id: u64 },
    /// Scene `index` (0-based) of `of` began; `cached` scenes are reused without rendering.
    Scene { id: u64, index: usize, of: usize, cached: bool },
    /// What the engine picked: rasterizer and encoder/output description.
    Engine { id: u64, renderer: String, output: String },
    /// Overall progress, 0..=1.
    Progress { id: u64, fraction: f32 },
    Problem { id: u64, scene: usize, message: String },
    Exporting { id: u64 },
    Done { id: u64, path: PathBuf, elapsed: Duration, duration: f64, rendered: usize, cached: usize },
    Failed { id: u64, scene: Option<usize>, error: String },
    Cancelled { id: u64 },
}

impl QueueEvent {
    pub fn id(&self) -> u64 {
        match self {
            QueueEvent::Started { id }
            | QueueEvent::Scene { id, .. }
            | QueueEvent::Engine { id, .. }
            | QueueEvent::Progress { id, .. }
            | QueueEvent::Problem { id, .. }
            | QueueEvent::Exporting { id }
            | QueueEvent::Done { id, .. }
            | QueueEvent::Failed { id, .. }
            | QueueEvent::Cancelled { id } => *id,
        }
    }
}

#[derive(Default)]
struct Shared {
    cancelled: Mutex<HashSet<u64>>,
    child: Mutex<Option<(u64, Child)>>,
    paused: AtomicBool,
}

pub struct RenderQueue {
    tx: mpsc::Sender<VideoRender>,
    events: async_channel::Receiver<QueueEvent>,
    shared: Arc<Shared>,
}

impl RenderQueue {
    pub fn spawn(worker: WorkerCommand) -> Self {
        let (tx, rx) = mpsc::channel::<VideoRender>();
        let (etx, events) = async_channel::unbounded();
        let shared = Arc::new(Shared::default());
        let s = shared.clone();
        std::thread::Builder::new()
            .name("studio-render-queue".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    while s.paused.load(Ordering::Relaxed) && !s.cancelled.lock().unwrap().contains(&job.id) {
                        std::thread::sleep(Duration::from_millis(150));
                    }
                    let id = job.id;
                    if s.cancelled.lock().unwrap().remove(&id) {
                        let _ = etx.send_blocking(QueueEvent::Cancelled { id });
                        continue;
                    }
                    let _ = etx.send_blocking(QueueEvent::Started { id });
                    let ev = run_job(&worker, &job, &s, &etx);
                    s.cancelled.lock().unwrap().remove(&id);
                    let _ = etx.send_blocking(ev);
                }
            })
            .expect("spawning the render queue");
        RenderQueue { tx, events, shared }
    }

    pub fn push(&self, job: VideoRender) {
        let _ = self.tx.send(job);
    }

    /// Cancel a waiting or running job (kills its engine process).
    pub fn cancel(&self, id: u64) {
        self.shared.cancelled.lock().unwrap().insert(id);
        if let Some((cid, child)) = self.shared.child.lock().unwrap().as_mut()
            && *cid == id {
                let _ = child.kill();
            }
    }

    /// Hold jobs that have not started yet (the running one finishes).
    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    pub fn events(&self) -> async_channel::Receiver<QueueEvent> {
        self.events.clone()
    }
}

impl Drop for RenderQueue {
    fn drop(&mut self) {
        if let Some((_, child)) = self.shared.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}

fn run_job(worker: &WorkerCommand, job: &VideoRender, s: &Shared, tx: &async_channel::Sender<QueueEvent>) -> QueueEvent {
    let id = job.id;
    let started = Instant::now();
    let fail = |scene: Option<usize>, error: String| QueueEvent::Failed { id, scene, error };
    let Some(ffmpeg) = crate::ffmpeg_path() else {
        return fail(None, "ffmpeg isn't set up yet. Download it in Settings → Rendering, then render again.".into());
    };
    let cache_dir = job.root.join(".cache");
    if let Err(e) = std::fs::create_dir_all(cache_dir.join("scenes")) {
        return fail(None, format!("creating {}: {e}", cache_dir.display()));
    }
    let style_file = cache_dir.join("style.json");
    if let Err(e) = std::fs::write(&style_file, &job.style_json) {
        return fail(None, format!("writing {}: {e}", style_file.display()));
    }
    let known: Vec<f64> = job.scenes.iter().filter_map(|s| s.expected).collect();
    let fallback = if known.is_empty() { 1.0 } else { known.iter().sum::<f64>() / known.len() as f64 };
    let weights: Vec<f64> = job.scenes.iter().map(|s| s.expected.unwrap_or(fallback).max(0.1)).collect();
    let total: f64 = weights.iter().sum();
    let of = job.scenes.len();
    let mut parts = Vec::with_capacity(of);
    let mut starts: Vec<f64> = Vec::with_capacity(of);
    let (mut rendered, mut cached, mut duration) = (0, 0, 0.0);
    let mut done_weight = 0.0;
    for (index, scene) in job.scenes.iter().enumerate() {
        if s.cancelled.lock().unwrap().contains(&id) {
            return QueueEvent::Cancelled { id };
        }
        let voiced = job.scenes.iter().any(|s| s.narration.is_some());
        let extra = match (&scene.narration, voiced) {
            (Some(n), _) if !n.parts.is_empty() => {
                n.parts.iter().filter_map(|(w, _)| w.file_name().map(|f| f.to_string_lossy().into_owned())).collect::<Vec<_>>().join("+")
            }
            (Some(n), _) => n.wav.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            (None, true) => "silence".into(),
            (None, false) => String::new(),
        };
        let key = cache::scene_key_with(&scene.path, &job.root, &job.style_json, job.output, &extra);
        let out = cache::scene_video(&job.root, &scene.path, key);
        let hit = out.is_file();
        let _ = tx.send_blocking(QueueEvent::Scene { id, index, of, cached: hit });
        if hit {
            cached += 1;
            let d = std::fs::read_to_string(out.with_extension("seconds"))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .or(scene.expected)
                .unwrap_or(0.0);
            starts.push(duration);
            duration += d;
        } else {
            let progress = |t: f64| {
                let w = weights[index];
                let within = scene.expected.map_or(0.5, |d| (t / d.max(1e-6)).clamp(0.0, 1.0));
                ((done_weight + w * within) / total) as f32
            };
            let voice = match (&scene.narration, voiced) {
                (Some(n), _) if !n.parts.is_empty() => Voice::Parts(n.parts.iter().map(|(w, _)| w.clone()).collect()),
                (Some(n), _) => Voice::Narration(n.wav.clone()),
                (None, true) => Voice::Silence,
                (None, false) => Voice::None,
            };
            match render_scene(worker, job, &scene.path, &style_file, &out, voice, s, tx, progress, index) {
                Ok(d) => {
                    rendered += 1;
                    let _ = std::fs::write(out.with_extension("seconds"), format!("{d}"));
                    starts.push(duration);
                    duration += d;
                    cache::prune(&job.root, &scene.path, &out);
                }
                Err(SceneError::Cancelled) => return QueueEvent::Cancelled { id },
                Err(SceneError::Failed(e)) => return fail(Some(index), e),
            }
        }
        done_weight += weights[index];
        let _ = tx.send_blocking(QueueEvent::Progress { id, fraction: (done_weight / total) as f32 });
        parts.push(out);
    }
    let _ = tx.send_blocking(QueueEvent::Exporting { id });
    // Scenes cross-dissolve into each other, so each starts a little earlier than the
    // sum of the ones before it.
    let lengths: Vec<f64> = starts.iter().enumerate().map(|(i, s)| starts.get(i + 1).copied().unwrap_or(duration) - s).collect();
    let fade = export::dissolve_for(&lengths);
    let starts: Vec<f64> = starts.iter().enumerate().map(|(i, s)| s - i as f64 * fade).collect();
    let duration = duration - fade * lengths.len().saturating_sub(1) as f64;
    let cues: Vec<crate::voice::Cue> = job
        .scenes
        .iter()
        .zip(&starts)
        .zip(&parts)
        .filter_map(|((s, start), video)| {
            let n = s.narration.as_ref()?;
            // Sentence by sentence where the engine placed them, when it wrote that down.
            let placed: Option<Vec<f64>> =
                std::fs::read_to_string(video.with_extension("starts")).ok().and_then(|t| serde_json::from_str(&t).ok());
            Some(match placed {
                Some(at) if !n.parts.is_empty() && at.len() == n.parts.len() => n
                    .parts
                    .iter()
                    .zip(&at)
                    .flat_map(|((wav, text), a)| crate::voice::cues(text, start + a, crate::voice::wav_seconds(wav).unwrap_or(0.0)))
                    .collect::<Vec<_>>(),
                _ => crate::voice::cues(&n.text, *start, n.seconds),
            })
        })
        .flatten()
        .collect();
    if let Some(dir) = job.export.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let srt = job.export.with_extension("srt");
    let burn = job.captions == Captions::Burned && !cues.is_empty();
    if job.captions != Captions::Off && !cues.is_empty()
        && let Err(e) = std::fs::write(&srt, crate::voice::srt(&cues)) {
            return fail(None, format!("writing captions: {e}"));
        }
    let finish = burn || job.watermark.is_some();
    let joined = if finish { job.export.with_extension("joined.mp4") } else { job.export.clone() };
    if let Err(e) = export::join_dissolve(&ffmpeg, &parts, &lengths, job.scenes.iter().any(|s| s.narration.is_some()), &joined) {
        return fail(None, format!("joining scenes: {e:#}"));
    }
    if finish {
        let font = job.fonts_dir.as_ref().map(|d| d.join("Inter-SemiBold.ttf")).filter(|p| p.is_file());
        let theme: serde_json::Value = serde_json::from_str(&job.style_json).unwrap_or_default();
        let color = |k: &str| theme["colors"][k].as_str().map(str::to_string);
        let r = export::finish(
            &ffmpeg,
            &joined,
            &export::Finish {
                captions: burn.then_some(srt.as_path()),
                watermark: job.watermark.as_ref(),
                font: font.as_deref(),
                text_rgb: &export::rgb(color("text").as_deref(), "FFFFFF"),
                shadow_rgb: &export::rgb(color("background").as_deref(), "000000"),
                width: job.output.width,
                height: job.output.height,
            },
            &job.export,
        );
        let _ = std::fs::remove_file(&joined);
        if let Err(e) = r {
            return fail(None, format!("{}: {e:#}", if burn { "adding captions" } else { "adding the watermark" }));
        }
    }
    QueueEvent::Done { id, path: job.export.clone(), elapsed: started.elapsed(), duration, rendered, cached }
}

enum Voice {
    None,
    Narration(PathBuf),
    /// Sentence WAVs, placed by the scene's `narrate(n)` calls.
    Parts(Vec<PathBuf>),
    Silence,
}

enum SceneError {
    Cancelled,
    Failed(String),
}

#[allow(clippy::too_many_arguments)]
fn render_scene(
    worker: &WorkerCommand,
    job: &VideoRender,
    script: &Path,
    style_file: &Path,
    out: &Path,
    voice: Voice,
    s: &Shared,
    tx: &async_channel::Sender<QueueEvent>,
    progress: impl Fn(f64) -> f32,
    index: usize,
) -> Result<f64, SceneError> {
    let id = job.id;
    // A plain partial name (older engines replaced only the last extension).
    let partial = {
        let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        out.with_file_name(format!("{stem}-partial.mp4"))
    };
    let _ = std::fs::remove_file(&partial);
    // Long, high-resolution scenes legitimately run for minutes; only stop real runaways.
    let pixels = (job.output.width as f64 * job.output.height as f64) / (1920.0 * 1080.0);
    let timeout = 600.0_f64.max(pixels * 60.0 * job.output.fps / 30.0 * 4.0);
    let mut cmd = Command::new(&worker.exe);
    // The engine finds ffmpeg through MANIM_FFMPEG first: point it at the same copy.
    if let Some(ff) = crate::ffmpeg_path() {
        cmd.env("MANIM_FFMPEG", ff);
    }
    cmd.args(&worker.args)
        .arg("--script")
        .arg(script)
        .arg("--root")
        .arg(&job.root)
        .arg("--timeout")
        .arg(format!("{timeout:.0}"))
        .arg("-r")
        .arg(format!("{}x{}", job.output.width, job.output.height))
        .arg("--fps")
        .arg(format!("{}", job.output.fps))
        .args(["--format", "mp4", "--progress", "json"])
        .args(["--renderer", job.backend.flag(), "--encoder", job.encoder.flag()])
        .arg("--theme")
        .arg(style_file)
        .arg("-o")
        .arg(&partial);
    if let Some(f) = &job.fonts_dir {
        cmd.arg("--fonts").arg(f);
    }
    match &voice {
        Voice::Narration(wav) => {
            cmd.arg("--narration").arg(wav);
        }
        Voice::Parts(wavs) => {
            for w in wavs {
                cmd.arg("--narration-part").arg(w);
            }
            cmd.arg("--narration-starts").arg(out.with_extension("starts"));
        }
        Voice::Silence => {
            cmd.arg("--silence");
        }
        Voice::None => {}
    }
    cmd.current_dir(&job.root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = match export::quiet(&mut cmd).spawn() {
        Ok(c) => c,
        Err(e) => return Err(SceneError::Failed(format!("starting the engine: {e}"))),
    };
    tie_to_app(&child);
    let stderr = child.stderr.take();
    *s.child.lock().unwrap() = Some((id, child));
    // A cancel that raced the spawn.
    if s.cancelled.lock().unwrap().contains(&id)
        && let Some((_, c)) = s.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    let mut error: Option<String> = None;
    let mut tail: Vec<String> = Vec::new();
    let mut duration = 0.0;
    let mut written: Option<PathBuf> = None;
    let mut encoder = String::new();
    if let Some(stderr) = stderr {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            match EngineEvent::parse(&line) {
                Some(EngineEvent::Start { renderer, output, .. }) => {
                    encoder = encoder_name(&output);
                    let _ = tx.send_blocking(QueueEvent::Engine { id, renderer: renderer_name(&renderer), output: encoder.clone() });
                }
                Some(EngineEvent::Error { message, .. }) => {
                    error.get_or_insert(message);
                }
                Some(EngineEvent::Problem { message, .. }) => {
                    let _ = tx.send_blocking(QueueEvent::Problem { id, scene: index, message });
                }
                Some(EngineEvent::Done { duration: d, outputs, .. }) => {
                    duration = d;
                    written = outputs.into_iter().map(PathBuf::from).find(|p| p.extension().is_some_and(|e| e == "mp4"));
                }
                Some(ev) => {
                    if let Some(t) = ev.time() {
                        let _ = tx.send_blocking(QueueEvent::Progress { id, fraction: progress(t) });
                    }
                }
                None => {
                    // `auto` reports its pick as a log line: "... → using CPU".
                    if let Some(pick) = line.split("using ").nth(1).filter(|_| line.contains("auto renderer")) {
                        let renderer = format!("{} (auto)", pick.trim());
                        let _ = tx.send_blocking(QueueEvent::Engine { id, renderer, output: encoder.clone() });
                        continue;
                    }
                    if !line.trim().is_empty() {
                        tail.push(line);
                        if tail.len() > 8 {
                            tail.remove(0);
                        }
                    }
                }
            }
        }
    }
    let taken = s.child.lock().unwrap().take();
    let status = taken.map(|(_, mut c)| c.wait());
    if s.cancelled.lock().unwrap().contains(&id) {
        let _ = std::fs::remove_file(&partial);
        return Err(SceneError::Cancelled);
    }
    let ok = matches!(status, Some(Ok(st)) if st.success());
    let video = written.filter(|p| p.is_file()).unwrap_or_else(|| partial.clone());
    if !ok || !video.is_file() {
        let _ = std::fs::remove_file(&video);
        let msg = error.unwrap_or_else(|| match status {
            Some(Ok(st)) if st.success() => "the engine finished but wrote no video".into(),
            Some(Ok(st)) => match tail.last() {
                Some(line) => format!("the engine exited with {st}: {line}"),
                None => format!("the engine exited with {st}"),
            },
            Some(Err(e)) => format!("the engine failed: {e}"),
            None => "the engine stopped".into(),
        });
        return Err(SceneError::Failed(msg));
    }
    if let Err(e) = std::fs::rename(&video, out) {
        return Err(SceneError::Failed(format!("saving {}: {e}", out.display())));
    }
    Ok(duration)
}

/// "CPU (tiny-skia/vello adaptive, 16 threads)" → "CPU"; "auto, calibrating: …" → "Auto".
fn renderer_name(s: &str) -> String {
    if s.starts_with("auto") {
        return "Auto".into();
    }
    s.split(" (").next().unwrap_or(s).trim().to_string()
}

/// "mp4 (h264_nvenc)" → "NVENC"; "mp4 (libx264)" → "x264".
fn encoder_name(s: &str) -> String {
    let inner = s.split('(').nth(1).map(|r| r.trim_end_matches(')')).unwrap_or(s);
    if inner.contains("nvenc") {
        "NVENC".into()
    } else if inner.contains("qsv") {
        "Quick Sync".into()
    } else if inner.contains("amf") {
        "AMF".into()
    } else if inner.contains("264") {
        "x264".into()
    } else {
        inner.to_string()
    }
}

/// Put an engine process in a job object that is killed when the app exits, so a
/// closed or crashed app never leaves renders running.
#[cfg(windows)]
fn tie_to_app(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        // Deliberately never closed: the handle closes when the app exits.
        job as usize
    });
    if job != 0 {
        unsafe {
            AssignProcessToJobObject(job as _, child.as_raw_handle() as _);
        }
    }
}

#[cfg(not(windows))]
fn tie_to_app(_child: &Child) {}
