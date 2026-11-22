//! Glue between the workspace and `studio-render`: the live stage preview, scene
//! thumbnails, and the render queue.

use crate::app::Workspace;
use crate::model::{Issue, JobStatus, Msg, RenderJob, Still, VideoStatus, VoiceLine, VoicePart};
use gpui::{AsyncApp, Context, RenderImage, WeakEntity, Window, prelude::FluentBuilder};
use smallvec::smallvec;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use studio_render::preview::{OpenScene, Preview, PreviewEvent, ThumbEvent, Thumbnailer};
use studio_render::queue::{QueueEvent, RenderQueue, SceneSpec, VideoRender, WorkerCommand};
use studio_render::{Backend, Encoder, Frame, Output, TimelineInfo};

pub const WM_MODES: [&str; 3] = ["Off", "Text", "Image"];
/// Text height as a fraction of the video height, and image width as a fraction of the
/// width, at size 100%.
pub const WM_TEXT_HEIGHT: f32 = 0.038;
pub const WM_IMAGE_WIDTH: f32 = 0.15;
/// Slider ranges: size, opacity, margin.
pub const WM_RANGES: [(f32, f32); 3] = [(0.4, 2.5), (0.1, 1.0), (0.0, 0.12)];
pub const WM_COLORS: [(&str, Option<&str>); 3] = [("Style", None), ("White", Some("FFFFFF")), ("Black", Some("000000"))];
const WM_IMAGE_TYPES: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// A narration track to mix: the sentences placed where the scene's timeline wants them.
struct MixPlan {
    wavs: Vec<PathBuf>,
    starts: Vec<f64>,
    dir: PathBuf,
}

/// Where a scene's narration sentences go on its timeline (`narrate(n)` marks): `Some`
/// when that differs from the current track, so a new one has to be mixed (off the UI
/// thread: `Workspace::mix_voice`).
fn plan_voice_mix(s: &crate::model::Scene, t: &TimelineInfo) -> Option<MixPlan> {
    let v = s.engine.voice.as_ref()?;
    if v.parts.is_empty() {
        return None;
    }
    let lines = v.lines();
    let starts = if studio_render::preview::uses_narrate(t) {
        studio_render::preview::narration_starts(t, &lines)
    } else {
        studio_render::voice::sequential_starts(&lines)
    };
    let same = starts.len() == v.starts.len() && starts.iter().zip(&v.starts).all(|(a, b)| (*a as f32 - b).abs() < 0.01);
    let dir = v.wav.parent().map(Path::to_path_buf)?;
    if same {
        return None;
    }
    Some(MixPlan { wavs: v.parts.iter().map(|p| p.wav.clone()).collect(), starts, dir })
}

/// Mixed tracks nothing points at any more are deleted once they are this old (a newer
/// mix may not be in use yet).
const TRACK_KEEP: Duration = Duration::from_secs(600);

/// Delete `track-*.wav` files in `dir` that are not in `keep` and are older than `TRACK_KEEP`.
fn prune_tracks(dir: &Path, keep: &[PathBuf]) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("track-") && name.ends_with(".wav")) || keep.contains(&p) {
            continue;
        }
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > TRACK_KEEP);
        if old {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Kokoro, the natural voice: downloaded once into data/voices/kokoro.
#[derive(Clone, Debug, PartialEq)]
pub enum NaturalVoice {
    Missing,
    Downloading(f32),
    Ready,
    Failed(String),
}

impl NaturalVoice {
    /// Point the voice layer at its folder and see whether the model is there.
    pub fn detect(quality: studio_render::kokoro::Quality) -> Self {
        if let Some(dir) = data_dir() {
            studio_render::voice::set_kokoro_dir(dir.join("voices").join("kokoro"));
        }
        studio_render::voice::set_kokoro_quality(quality);
        // Only the chosen model stays on disk once it's there.
        if let Some(dir) = studio_render::voice::kokoro_dir() {
            studio_render::kokoro::remove_other_models(dir, quality);
        }
        if studio_render::voice::kokoro_ready() { Self::Ready } else { Self::Missing }
    }
}
use studio_ui::IconName;

/// How often the "working" line is redrawn.
pub const WORKING_TICK_MS: u64 = 250;

/// Live preview resolution and rate (the stage is ~800–1200 device px wide).
pub const PREVIEW: Output = Output::new(1280, 720, 30.0);
const THUMB: Output = Output::new(384, 216, 30.0);

/// Output presets, in the order Settings → Rendering lists them.
pub const OUTPUTS: [(&str, Output); 5] = [
    ("1080p · 30 fps", Output::new(1920, 1080, 30.0)),
    ("1080p · 60 fps", Output::new(1920, 1080, 60.0)),
    ("720p · 30 fps", Output::new(1280, 720, 30.0)),
    ("4K · 30 fps", Output::new(3840, 2160, 30.0)),
    ("Vertical 1080×1920", Output::new(1080, 1920, 30.0)),
];
pub const BACKENDS: [(&str, Backend); 3] = [("Automatic", Backend::Auto), ("GPU", Backend::Gpu), ("CPU", Backend::Cpu)];
pub const ENCODERS: [(&str, Encoder); 2] = [("Hardware when available", Encoder::Auto), ("Software (x264)", Encoder::X264)];
pub const SPEEDS: [(&str, f32); 4] = [("0.9×", 0.9), ("1.0×", 1.0), ("1.1×", 1.1), ("1.2×", 1.2)];
/// Lecture slides saved beside the video: what the menus show, in `Formats` order below.
pub const SLIDES: [&str; 4] = ["No slides", "Slides as PDF", "Slides as PowerPoint", "PDF and PowerPoint slides"];

pub fn slide_formats(choice: usize) -> studio_render::slides::Formats {
    studio_render::slides::Formats { pdf: matches!(choice, 1 | 3), pptx: matches!(choice, 2 | 3) }
}

pub const CAPTIONS: [&str; 3] = ["Off", ".srt file", "Burned in + .srt"];

pub struct Engine {
    pub preview: Preview,
    pub thumbs: Thumbnailer,
    pub queue: RenderQueue,
    pub fonts_dir: Option<PathBuf>,
    pub next_job: u64,
    /// What the engine picked on the last render: "GPU (…) · NVENC" etc.
    pub backend_info: Option<String>,
    pub ffmpeg: bool,
}

/// What the stage shows.
#[derive(Default)]
pub struct Stage {
    /// Generation of the scene the preview is showing (older events are stale).
    pub generation: u64,
    /// (video, scene index, script, style) the preview has open.
    pub open: Option<((usize, usize), usize, PathBuf, usize)>,
    pub frame: Option<Arc<RenderImage>>,
    pub timeline: Option<TimelineInfo>,
    pub error: Option<String>,
    pub loading: bool,
    pub load_ms: Option<u32>,
    /// The Code view's file, keyed by path and modification time.
    pub code: Option<(PathBuf, Option<std::time::SystemTime>, std::rc::Rc<str>)>,
    /// Latest narration mix asked for per (video, scene): older results are dropped.
    pub mix_latest: std::collections::HashMap<((usize, usize), usize), u64>,
    pub mix_seq: u64,
    /// Videos whose thumbnails wait for their narration (it changes scene lengths).
    pub thumbs_deferred: std::collections::HashSet<(usize, usize)>,
    /// Videos whose render waits for their narration to be spoken.
    pub render_after_voice: std::collections::HashSet<(usize, usize)>,
    /// The previous scene's last frame, dissolving out over the next scene during
    /// playback (the same dissolve the export uses), and when it started.
    pub fade_from: Option<(Arc<RenderImage>, std::time::Instant)>,
    /// Images nothing shows any more (dropped scenes, cleared frames), freed from the
    /// GPU cache at the next render (`Workspace::flush_dead_images`). Never free one
    /// straight away with `cx.drop_image`: gpui can present the last drawn scene again
    /// without redrawing it (a repaint, fast mouse input), and a sprite in that scene
    /// pointing at a freed atlas texture panics in `directx_atlas.rs`.
    pub dead: Vec<Arc<RenderImage>>,
}

impl Stage {
    /// Opacity of the dissolving frame, if one is still showing.
    pub fn fade_opacity(&self) -> Option<f32> {
        let (_, t0) = self.fade_from.as_ref()?;
        let f = t0.elapsed().as_secs_f32() / studio_render::export::DISSOLVE as f32;
        (f < 1.0).then_some(1.0 - f)
    }
}

fn data_dir() -> Option<PathBuf> {
    crate::store::data_dir()
}

/// The engine worker: a console-subsystem helper next to the app when present (so
/// ffmpeg never flashes a console window in release builds), else the app itself.
fn worker_command() -> WorkerCommand {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("manim-studio"));
    let helper = exe.with_file_name(if cfg!(windows) { "studio-engine.exe" } else { "studio-engine" });
    if helper.is_file() {
        WorkerCommand { exe: helper, args: vec![] }
    } else {
        WorkerCommand { exe, args: vec![studio_render::worker::ARG.to_string()] }
    }
}

fn bundled_fonts() -> Vec<(String, std::borrow::Cow<'static, [u8]>)> {
    studio_ui::assets::Assets::iter()
        .filter(|p| p.ends_with(".ttf"))
        .filter_map(|p| {
            let name = p.rsplit('/').next().unwrap_or(&p).to_string();
            studio_ui::assets::Assets::get(&p).map(|f| (name, f.data))
        })
        .collect()
}

impl Engine {
    pub fn start() -> Self {
        let fonts = bundled_fonts();
        // Downloaded tools (ffmpeg) live in <data>/tools; set before looking for ffmpeg.
        if let Some(d) = data_dir() {
            studio_render::tools::set_dir(d.join("tools"));
        }
        studio_render::fonts::register_data(fonts.iter().map(|(_, b)| b.as_ref()));
        let fonts_dir = data_dir().map(|d| d.join("engine-fonts")).filter(|dir| {
            studio_render::fonts::materialize(dir, fonts.iter().map(|(n, b)| (n.as_str(), b.as_ref()))).is_ok()
        });
        Engine {
            preview: Preview::spawn(PREVIEW),
            thumbs: Thumbnailer::spawn(THUMB),
            queue: RenderQueue::spawn(worker_command()),
            fonts_dir,
            next_job: 1,
            backend_info: None,
            ffmpeg: studio_render::ffmpeg_path().is_some(),
        }
    }
}

pub fn to_image(frame: Frame) -> Arc<RenderImage> {
    let Frame { width, height, bgra } = frame;
    let buf = image::RgbaImage::from_raw(width, height, bgra).expect("frame size matches its buffer");
    Arc::new(RenderImage::new(smallvec![image::Frame::new(buf)]))
}

fn issue_for(timeline: &TimelineInfo) -> Option<Issue> {
    timeline.problems.first().map(|p| Issue::Layout { count: timeline.problems.len(), first: p.message.clone() })
}

fn error_issue(error: &str) -> Issue {
    Issue::Error { message: studio_render::short_error(error), line: studio_render::error_line(error) }
}

/// Thumbnail keys with this bit are house-style previews, not scenes.
const STYLE_KEY: u64 = 1 << 62;
/// Thumbnail keys with this bit are house-style preview clips.
const STYLE_CLIP_KEY: u64 = 1 << 61;
/// Frame rate of the style preview clips (enough for a 4 s title card that loops).
pub const STYLE_CLIP_FPS: f64 = 12.0;
/// Clips kept in memory at once (each is ~50 frames of 384×216).
const STYLE_CLIPS_KEPT: usize = 2;

/// Scene thumbnail keys: generation (bits 48–59), project (40–47), video (20–39), scene.
/// The generation changes whenever videos move, so results for old positions are dropped.
fn thumb_key(generation: u64, video: (usize, usize), scene: usize) -> u64 {
    ((generation & 0xFFF) << 48) | (((video.0 as u64) & 0xFF) << 40) | ((video.1 as u64) << 20) | scene as u64
}

/// The user's Videos folder, where finished videos are saved (`Videos/Manim Studio`).
pub fn videos_dir() -> Option<PathBuf> {
    directories::UserDirs::new().and_then(|d| d.video_dir().map(|p| p.join("Manim Studio")))
}

/// Text made safe for a file name: readable, minus what file systems reject.
fn file_name_part(text: &str) -> String {
    let s: String = text
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // Windows drops trailing dots and spaces from names.
    s.trim_end_matches(['.', ' ']).to_string()
}

/// Where a finished video goes: `<Videos>/<project>/<title>.mp4`, where a teacher can find
/// it, else (no Videos folder) the video's own `exports` folder. `tag` tells apart two
/// videos of one project with the same title.
pub fn export_path(videos: Option<&Path>, root: &Path, project: &str, title: &str, tag: Option<&str>) -> PathBuf {
    let mut name = file_name_part(title);
    if name.is_empty() {
        name = "Video".into();
    }
    if let Some(tag) = tag {
        name = format!("{name} ({tag})");
    }
    let dir = match videos {
        Some(v) => {
            let project = file_name_part(project);
            if project.is_empty() { v.to_path_buf() } else { v.join(project) }
        }
        None => root.join("exports"),
    };
    dir.join(format!("{name}.mp4"))
}

/// Where a saved video is, in the words a file manager uses: "Videos › Manim Studio ›
/// My videos" for the Videos folder, else the folder's own name.
fn saved_place(path: &Path) -> String {
    let dir = path.parent().unwrap_or(path);
    if let Some(videos) = videos_dir()
        && let Ok(rel) = dir.strip_prefix(&videos)
    {
        let mut parts = vec!["Videos".to_string(), "Manim Studio".to_string()];
        parts.extend(rel.iter().map(|p| p.to_string_lossy().into_owned()));
        return parts.join(" › ");
    }
    dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string())
}

/// What "reveal in the file manager" is called on this platform.
pub const REVEAL_LABEL: &str = if cfg!(windows) {
    "Show in Explorer"
} else if cfg!(target_os = "macos") {
    "Show in Finder"
} else {
    "Show in file manager"
};

/// Reveal a file in the system file manager.
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer wants `/select,"C:\a b\c.mp4"`. `arg` would quote the whole thing
        // (`"/select,C:\a b\c.mp4"`) once the path has a space, and Explorer then opens
        // Documents instead.
        let _ = std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", studio_render::plain_path(path)))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}

/// Open a file with its default app (scene scripts in the user's editor).
pub fn open_file(path: &Path) {
    #[cfg(windows)]
    // Explorer opens a file with its default app. (Not `cmd /C start`: cmd would parse
    // characters such as & in a file name the assistant chose.)
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

impl Workspace {
    /// Forward engine events to the workspace for as long as it lives.
    pub fn listen_to_engine(&mut self, cx: &mut Context<Self>) {
        let preview = self.engine.preview.events();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Ok(ev) = preview.recv().await {
                if this.update(cx, |this, cx| this.on_preview(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let thumbs = self.engine.thumbs.events();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Ok(ev) = thumbs.recv().await {
                if this.update(cx, |this, cx| this.on_thumb(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let queue = self.engine.queue.events();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Ok(ev) = queue.recv().await {
                if this.update(cx, |this, cx| this.on_queue(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// The house style a video renders in (its own, else the default).
    pub fn video_style(&self, video: (usize, usize)) -> usize {
        let own = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)).and_then(|v| v.settings.style);
        own.unwrap_or(self.prefs.house_style).min(studio_render::style::STYLES.len() - 1)
    }

    /// The output preset a video renders at (its own, else the default).
    pub fn video_output(&self, video: (usize, usize)) -> usize {
        let own = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)).and_then(|v| v.settings.output);
        own.unwrap_or(self.prefs.output).min(OUTPUTS.len() - 1)
    }

    /// The lecture-slide choice for a video (its own, else the default).
    pub fn video_slides(&self, video: (usize, usize)) -> usize {
        let own = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)).and_then(|v| v.settings.slides);
        own.unwrap_or(self.prefs.slides).min(SLIDES.len() - 1)
    }

    pub fn set_video_slides(&mut self, slides: usize, cx: &mut Context<Self>) {
        let video = self.current;
        let Some(v) = self.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)) else { return };
        v.settings.slides = Some(slides);
        self.save_video_settings(video);
        cx.notify();
    }

    /// Picker for the current video's lecture slides (strip and storyboard).
    pub fn slides_menu(&self, key: &'static str, upward: bool, cx: &mut Context<Self>) -> studio_ui::components::Dropdown {
        use crate::app::with_value;
        use studio_ui::components::{Button, Dropdown, MenuItem};
        let choice = self.video_slides(self.current);
        Dropdown::new(
            key,
            Button::new((key, 0usize), SLIDES[choice]).small().ghost().with_icon(IconName::Doc).icon_right(IconName::ChevronDown),
            SLIDES.iter().map(|n| MenuItem::new(*n)).collect(),
        )
        .selected(choice)
        .width(240.)
        .when(upward, |d| d.upward())
        .open(self.menu_open(key))
        .on_toggle(with_value(cx, move |this, open: bool, _, cx| this.toggle_menu(key, open, cx)))
        .on_select(with_value(cx, |this, i: usize, _, cx| {
            this.open_menu = None;
            this.set_video_slides(i, cx);
        }))
    }

    fn save_video_settings(&self, video: (usize, usize)) {
        // The whole file: video.json also holds the title, status, storyboard and session.
        if let Some(v) = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)) {
            crate::store::save_video(v);
        }
    }

    pub fn set_video_style(&mut self, style: usize, cx: &mut Context<Self>) {
        let video = self.current;
        let Some(v) = self.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)) else { return };
        v.settings.style = Some(style);
        self.save_video_settings(video);
        self.restyle(cx);
    }

    pub fn set_video_output(&mut self, output: usize, cx: &mut Context<Self>) {
        let video = self.current;
        let Some(v) = self.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)) else { return };
        v.settings.output = Some(output);
        self.save_video_settings(video);
        cx.notify();
    }

    /// Pickers for the current video's house style and output (small, for toolbars).
    pub fn video_setting_menus(
        &self,
        keys: (&'static str, &'static str),
        upward: bool,
        cx: &mut Context<Self>,
    ) -> (studio_ui::components::Dropdown, studio_ui::components::Dropdown) {
        use crate::app::with_value;
        use studio_ui::components::{Button, Dropdown, MenuItem};
        let video = self.current;
        let style = self.video_style(video);
        let output = self.video_output(video);
        let styles = studio_render::style::STYLES;
        let (sk, ok) = keys;
        let style_menu = Dropdown::new(
            sk,
            Button::new((sk, 0usize), styles[style].name).small().ghost().with_icon(IconName::Palette).icon_right(IconName::ChevronDown),
            styles.iter().map(|s| MenuItem::new(s.name)).collect(),
        )
        .selected(style)
        .width(200.)
        .when(upward, |d| d.upward())
        .open(self.menu_open(sk))
        .on_toggle(with_value(cx, move |this, open: bool, _, cx| this.toggle_menu(sk, open, cx)))
        .on_select(with_value(cx, |this, i: usize, _, cx| {
            this.open_menu = None;
            this.set_video_style(i, cx);
        }));
        let output_menu = Dropdown::new(
            ok,
            Button::new((ok, 0usize), OUTPUTS[output].0).small().ghost().with_icon(IconName::Film).icon_right(IconName::ChevronDown),
            OUTPUTS.iter().map(|(n, _)| MenuItem::new(*n)).collect(),
        )
        .selected(output)
        .width(220.)
        .when(upward, |d| d.upward())
        .open(self.menu_open(ok))
        .on_toggle(with_value(cx, move |this, open: bool, _, cx| this.toggle_menu(ok, open, cx)))
        .on_select(with_value(cx, |this, i: usize, _, cx| {
            this.open_menu = None;
            this.set_video_output(i, cx);
        }));
        (style_menu, output_menu)
    }

    fn open_request(&self, video: (usize, usize), scene: usize) -> Option<OpenScene> {
        self.open_request_styled(video, scene, self.video_style(video))
    }

    fn open_request_styled(&self, video: (usize, usize), scene: usize, style: usize) -> Option<OpenScene> {
        let v = self.projects.get(video.0)?.videos.get(video.1)?;
        let file = v.scenes.get(scene)?.engine.file.clone()?;
        Some(OpenScene {
            path: file,
            root: v.dir.clone()?,
            theme_json: studio_render::style::json(style).to_string(),
            scene: None,
            globals: vec![],
            narration: v.scenes.get(scene)?.engine.voice.as_ref().map(|l| l.seconds as f64),
            narration_lines: v.scenes.get(scene)?.engine.voice.as_ref().map(VoiceLine::lines).unwrap_or_default(),
            narration_cues: v.scenes.get(scene)?.engine.voice.as_ref().map(|l| l.cues.clone()).unwrap_or_default(),
        })
    }

    pub fn voice_quality(&self) -> studio_render::kokoro::Quality {
        use studio_render::kokoro::Quality;
        Quality::ALL[self.prefs.voice_model.min(Quality::ALL.len() - 1)]
    }

    /// The picked natural-voice model is downloaded.
    pub fn voice_quality_installed(&self) -> bool {
        let q = self.prefs.voice_model;
        self.probes.borrow_mut().kokoro_installed.get(q, |_| {
            studio_render::voice::kokoro_dir().is_some_and(|d| studio_render::kokoro::installed(d, self.voice_quality()))
        })
    }

    /// Pick the natural-voice model: re-speak with it, downloading it first if needed.
    pub fn set_voice_quality(&mut self, i: usize, cx: &mut Context<Self>) {
        self.prefs.voice_model = i;
        self.save_prefs();
        studio_render::voice::set_kokoro_quality(self.voice_quality());
        if self.voice_quality_installed() {
            self.refresh_voice(self.current, cx);
        } else if !matches!(self.natural, NaturalVoice::Downloading(_)) {
            self.download_natural_voice(cx);
        }
        cx.notify();
    }

    /// Fetch espeak-ng into Studio's data folder (Windows), then re-speak the open video so
    /// unknown words are read instead of spelled.
    pub fn install_espeak(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = studio_render::voice::kokoro_dir().map(Path::to_path_buf) else { return };
        if matches!(self.espeak, NaturalVoice::Downloading(_)) {
            return;
        }
        self.espeak = NaturalVoice::Downloading(0.);
        let cancel = Arc::new(AtomicBool::new(false));
        self.espeak_cancel = cancel.clone();
        cx.notify();
        let (tx, rx) = async_channel::unbounded::<f32>();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let task = cx.background_executor().spawn(async move {
                let mut last = 0.0f32;
                studio_render::kokoro::espeak::install(&dir, &cancel, move |done, total| {
                    let f = done as f32 / total.max(1) as f32;
                    if f - last >= 0.02 || done == total {
                        last = f;
                        let _ = tx.try_send(f);
                    }
                })
            });
            while let Ok(f) = rx.recv().await {
                this.update(cx, |this, cx| {
                    this.espeak = NaturalVoice::Downloading(f);
                    cx.notify();
                })
                .ok();
            }
            let result = task.await;
            this.update(cx, |this, cx| {
                this.probes.get_mut().clear();
                match result {
                    Ok(()) => {
                        this.espeak = NaturalVoice::Ready;
                        studio_render::voice::reset_kokoro();
                        this.refresh_voice(this.current, cx);
                    }
                    // Cancelled by the teacher: not a failure.
                    Err(e) if studio_render::is_cancelled(&e) => this.espeak = NaturalVoice::Missing,
                    Err(e) => this.espeak = NaturalVoice::Failed(format!("{e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The narration voice and speed, as shown in the storyboard panel.
    pub fn voice_label(&self) -> String {
        if !self.prefs.narration {
            return "Off".into();
        }
        if !self.probes.borrow_mut().kokoro_usable.get((), |_| studio_render::voice::kokoro_ready()) {
            return "Natural voice not downloaded".into();
        }
        let id = self.voice_spec().voice_id().to_string();
        let name = studio_render::kokoro::VOICES.iter().find(|(v, _)| *v == id).map_or(id.clone(), |(_, l)| l.to_string());
        format!("{name} · {}", SPEEDS[self.prefs.speed.min(SPEEDS.len() - 1)].0)
    }

    /// The default watermark from Settings, if it is set up (ignores projects).
    pub fn default_watermark(&self) -> Option<studio_render::export::Watermark> {
        use studio_render::export::{Mark, Watermark};
        let w = &self.prefs.watermark;
        let mark = match w.mode {
            1 if !w.text.trim().is_empty() => Mark::Text(w.text.trim().to_string()),
            2 => Mark::Image(w.image.clone().filter(|p| p.is_file())?),
            _ => return None,
        };
        use studio_render::export::Align;
        let base = if matches!(mark, Mark::Text(_)) { WM_TEXT_HEIGHT } else { WM_IMAGE_WIDTH };
        let a = w.anchor.min(8);
        Some(Watermark {
            mark,
            at: (Align::from_index(a % 3), Align::from_index(a / 3)),
            size: base * w.scale.clamp(WM_RANGES[0].0, WM_RANGES[0].1),
            opacity: w.alpha.clamp(WM_RANGES[1].0, WM_RANGES[1].1),
            margin: w.margin.clamp(WM_RANGES[2].0, WM_RANGES[2].1),
            color: WM_COLORS[w.color.min(WM_COLORS.len() - 1)].1.map(str::to_string),
            shadow: w.shadow,
        })
    }

    /// The watermark for videos in project `p`.
    pub fn watermark_for(&self, p: usize) -> Option<studio_render::export::Watermark> {
        if self.projects.get(p).is_none_or(|p| p.no_watermark) {
            return None;
        }
        self.default_watermark()
    }

    /// Set watermark slider `ix` (size, opacity, margin) from window x-coordinate `x`.
    pub fn set_wm_slider(&mut self, ix: usize, x: gpui::Pixels, cx: &mut Context<Self>) {
        let b = self.wm_bounds[ix].get();
        if b.size.width <= gpui::px(0.) {
            return;
        }
        let f = ((x - b.origin.x) / b.size.width).clamp(0., 1.);
        let (lo, hi) = WM_RANGES[ix];
        let v = lo + f * (hi - lo);
        let w = &mut self.prefs.watermark;
        match ix {
            0 => w.scale = v,
            1 => w.alpha = v,
            _ => w.margin = v,
        }
        cx.notify();
    }

    /// The house style's text and background colours (`RRGGBB`) for the current video.
    pub fn style_rgb(&self) -> (String, String) {
        let theme: serde_json::Value = serde_json::from_str(studio_render::style::json(self.video_style(self.current))).unwrap_or_default();
        let c = |k: &str, d: &str| studio_render::export::rgb(theme["colors"][k].as_str(), d);
        (c("text", "FFFFFF"), c("background", "000000"))
    }

    /// Turn the default watermark off (or back on) for the current project.
    pub fn set_project_watermark(&mut self, off: bool, cx: &mut Context<Self>) {
        if let Some(p) = self.projects.get_mut(self.current.0) {
            p.no_watermark = off;
            crate::store::save_project(p);
        }
        cx.notify();
    }

    /// Pick the watermark image; it is copied into the data folder.
    pub fn pick_watermark_image(&mut self, cx: &mut Context<Self>) {
        self.pick("Choose a watermark image (PNG with transparency works best)", cx, |this, paths, cx| {
            let Some(src) = paths.into_iter().next() else { return };
            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if !WM_IMAGE_TYPES.contains(&ext.as_str()) {
                this.notice = Some("Pick a PNG, JPEG or WebP image for the watermark.".into());
                cx.notify();
                return;
            }
            let Some(dir) = data_dir().map(|d| d.join("watermark")) else { return };
            let _ = std::fs::remove_dir_all(&dir);
            let dest = dir.join(src.file_name().unwrap_or_default());
            match std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&src, &dest)) {
                Ok(_) => {
                    this.prefs.watermark.image = Some(dest);
                    this.prefs.watermark.mode = 2;
                    this.save_prefs();
                }
                Err(e) => this.notice = Some(format!("Couldn't use that image: {e}")),
            }
            cx.notify();
        });
    }

    /// Per-project watermark menu (strip and storyboard).
    pub fn watermark_menu(&self, key: &'static str, upward: bool, cx: &mut Context<Self>) -> studio_ui::components::Dropdown {
        use crate::app::with_value;
        use studio_ui::components::{Button, Dropdown, MenuItem};
        let off = self.projects.get(self.current.0).is_some_and(|p| p.no_watermark);
        let set_up = self.default_watermark().is_some();
        let label = match (set_up, off) {
            (false, _) => "No watermark",
            (true, true) => "Watermark off",
            (true, false) => "Watermark",
        };
        let first = if set_up { "Use the default watermark" } else { "Default watermark (set it up in Settings)" };
        Dropdown::new(
            key,
            Button::new((key, 0usize), label).small().ghost().with_icon(IconName::Image).icon_right(IconName::ChevronDown),
            vec![MenuItem::new(first), MenuItem::new("No watermark in this project"), MenuItem::new("Watermark settings…")],
        )
        .selected(if off { 1 } else { 0 })
        .width(280.)
        .when(upward, |d| d.upward())
        .open(self.menu_open(key))
        .on_toggle(with_value(cx, move |this, open: bool, _, cx| this.toggle_menu(key, open, cx)))
        .on_select(with_value(cx, |this, i: usize, _, cx| {
            this.open_menu = None;
            match i {
                0 => this.set_project_watermark(false, cx),
                1 => this.set_project_watermark(true, cx),
                _ => {
                    this.settings_page = crate::app::SettingsPage::Rendering;
                    this.go(crate::app::Screen::Settings, cx);
                }
            }
        }))
    }

    /// Download ffmpeg into the data folder (first launch, or from Settings when the
    /// automatic download failed). Renders find it straight away.
    pub fn download_ffmpeg(&mut self, cx: &mut Context<Self>) {
        if matches!(self.ffmpeg_dl, NaturalVoice::Downloading(_)) {
            return;
        }
        self.ffmpeg_dl = NaturalVoice::Downloading(0.);
        let cancel = Arc::new(AtomicBool::new(false));
        self.ffmpeg_cancel = cancel.clone();
        cx.notify();
        let (tx, rx) = async_channel::unbounded::<f32>();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let task = cx.background_executor().spawn(async move {
                let mut last = 0.0f32;
                studio_render::tools::download_ffmpeg(&cancel, move |done, total| {
                    let f = done as f32 / total.max(1) as f32;
                    if f - last >= 0.01 || done == total {
                        last = f;
                        let _ = tx.try_send(f);
                    }
                })
            });
            while let Ok(f) = rx.recv().await {
                this.update(cx, |this, cx| {
                    this.ffmpeg_dl = NaturalVoice::Downloading(f);
                    cx.notify();
                })
                .ok();
            }
            let result = task.await;
            this.update(cx, |this, cx| {
                this.probes.get_mut().clear();
                match result {
                    Ok(()) => {
                        this.ffmpeg_dl = NaturalVoice::Ready;
                        this.engine.ffmpeg = true;
                    }
                    Err(e) if studio_render::is_cancelled(&e) => this.ffmpeg_dl = NaturalVoice::Missing,
                    Err(e) => this.ffmpeg_dl = NaturalVoice::Failed(format!("{e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Download Kokoro (model, voices, dictionaries), then re-speak the open video with it.
    pub fn download_natural_voice(&mut self, cx: &mut Context<Self>) {
        if !studio_render::kokoro::supported() {
            return;
        }
        let Some(dir) = studio_render::voice::kokoro_dir().map(Path::to_path_buf) else { return };
        if matches!(self.natural, NaturalVoice::Downloading(_)) {
            return;
        }
        self.natural = NaturalVoice::Downloading(0.);
        let cancel = Arc::new(AtomicBool::new(false));
        self.natural_cancel = cancel.clone();
        let quality = self.voice_quality();
        cx.notify();
        let (tx, rx) = async_channel::unbounded::<f32>();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let task = cx.background_executor().spawn(async move {
                let mut last = 0.0f32;
                studio_render::kokoro::download(&dir, quality, &cancel, move |done, total| {
                    let f = done as f32 / total.max(1) as f32;
                    if f - last >= 0.01 || done == total {
                        last = f;
                        let _ = tx.try_send(f);
                    }
                })
            });
            while let Ok(f) = rx.recv().await {
                this.update(cx, |this, cx| {
                    this.natural = NaturalVoice::Downloading(f);
                    cx.notify();
                })
                .ok();
            }
            let result = task.await;
            this.update(cx, |this, cx| {
                this.probes.get_mut().clear();
                match result {
                    Ok(()) => {
                        this.natural = NaturalVoice::Ready;
                        // Onboarding shows its own "Ready".
                        if this.screen != crate::app::Screen::Onboarding {
                            this.notice = Some("The natural voice is installed. Narration is ready.".into());
                        }
                        if let Some(dir) = studio_render::voice::kokoro_dir() {
                            studio_render::kokoro::remove_other_models(dir, quality);
                        }
                        this.refresh_voice(this.current, cx);
                    }
                    Err(e) if studio_render::is_cancelled(&e) => {
                        // Cancelled by the teacher: back to where it was, no error.
                        this.natural = if studio_render::voice::kokoro_ready() { NaturalVoice::Ready } else { NaturalVoice::Missing };
                    }
                    Err(e) => {
                        // A failed upgrade still leaves the other model usable.
                        this.natural = if studio_render::voice::kokoro_ready() {
                            this.notice = Some(format!("The voice download failed: {e:#}"));
                            NaturalVoice::Ready
                        } else {
                            NaturalVoice::Failed(format!("{e:#}"))
                        };
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stop the natural-voice download (partial files stay, so the next one continues).
    pub fn cancel_natural_voice(&mut self, cx: &mut Context<Self>) {
        self.natural_cancel.store(true, Ordering::Relaxed);
        cx.notify();
    }

    pub fn cancel_espeak(&mut self, cx: &mut Context<Self>) {
        self.espeak_cancel.store(true, Ordering::Relaxed);
        cx.notify();
    }

    pub fn cancel_ffmpeg(&mut self, cx: &mut Context<Self>) {
        self.ffmpeg_cancel.store(true, Ordering::Relaxed);
        cx.notify();
    }

    /// The Kokoro voice id narration uses.
    pub fn voice_spec_id(&self) -> &'static str {
        let spec = self.voice_spec();
        studio_render::kokoro::VOICES.iter().map(|(id, _)| *id).find(|id| *id == spec.voice_id()).unwrap_or(studio_render::kokoro::DEFAULT_VOICE)
    }

    fn voice_spec(&self) -> studio_render::voice::VoiceSpec {
        studio_render::voice::VoiceSpec { voice: self.prefs.voice.clone(), rate: SPEEDS[self.prefs.speed.min(SPEEDS.len() - 1)].1 }
    }

    /// Speak every scene's narration line of `video` in the background (cached per line),
    /// then re-run what depends on scene lengths.
    pub fn refresh_voice(&mut self, video: (usize, usize), cx: &mut Context<Self>) {
        let Some(v) = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)) else { return };
        let Some(dir) = v.dir.clone() else {
            self.flush_deferred_thumbs(video);
            self.voice_settled(video, cx);
            return;
        };
        let enabled = self.prefs.narration && studio_render::kokoro::supported();
        let lines: Vec<(usize, String)> =
            v.scenes.iter().enumerate().map(|(i, s)| (i, s.say.trim().to_string())).collect();
        // Every refresh supersedes the ones before it: their results are dropped.
        let generation = match self.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)) {
            Some(v) => {
                v.voice_gen = v.voice_gen.wrapping_add(1);
                v.voice_gen
            }
            None => return,
        };
        if !enabled {
            let mut changed = false;
            if let Some(v) = self.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)) {
                for s in &mut v.scenes {
                    changed |= s.engine.voice.take().is_some();
                }
            }
            if changed {
                self.after_voice_change(video, cx);
            } else {
                self.flush_deferred_thumbs(video);
            }
            self.voice_settled(video, cx);
            return;
        }
        let spec = self.voice_spec();
        let cache = studio_render::voice::cache_dir(&dir);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let spoken = cx
                .background_executor()
                .spawn(async move {
                    lines
                        .into_iter()
                        .map(|(i, text)| {
                            if text.is_empty() {
                                return (i, text, Ok(None));
                            }
                            // Each sentence on its own, so the scene can place it.
                            let line = studio_render::voice::speak(&text, &spec, &cache)
                                .and_then(|spoken| {
                                    if spoken.sentences.is_empty() {
                                        return Ok(None);
                                    }
                                    let lines: Vec<f64> = spoken.sentences.iter().map(|s| s.seconds).collect();
                                    let starts = studio_render::voice::sequential_starts(&lines);
                                    let wavs: Vec<PathBuf> = spoken.sentences.iter().map(|s| s.wav.clone()).collect();
                                    let track = studio_render::voice::compose(&wavs, &starts, &cache)?;
                                    Ok(Some((track, spoken, starts)))
                                })
                                .map_err(|e| format!("{e:#}"));
                            (i, text, line)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                let mut changed = false;
                let mut first_error: Option<(usize, String)> = None;
                if let Some(v) = this.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)) {
                    // A newer refresh (other voice, speed, order) owns the result now.
                    if v.voice_gen != generation {
                        return;
                    }
                    for (i, text, line) in spoken {
                        let Some(s) = v.scenes.get_mut(i) else { continue };
                        if s.say.trim() != text {
                            continue;
                        }
                        let line = match line {
                            Ok(l) => l,
                            Err(e) => {
                                first_error.get_or_insert((i, e));
                                continue;
                            }
                        };
                        let new = line.map(|(l, spoken, starts)| VoiceLine {
                            wav: l.wav,
                            seconds: l.seconds as f32,
                            text,
                            parts: spoken.sentences.into_iter().map(|s| VoicePart { wav: s.wav, seconds: s.seconds as f32, text: s.text }).collect(),
                            starts: starts.iter().map(|s| *s as f32).collect(),
                            cues: spoken.cues,
                        });
                        if s.engine.voice != new {
                            s.engine.voice = new;
                            changed = true;
                        }
                    }
                }
                if changed {
                    this.after_voice_change(video, cx);
                } else {
                    this.flush_deferred_thumbs(video);
                }
                // Say so when speech fails (not while the voice just isn't downloaded:
                // that has its own prompt).
                if let Some((i, e)) = first_error
                    && studio_render::voice::kokoro_ready()
                {
                    this.notice = Some(format!("Couldn't speak chapter {}'s narration: {}", i + 1, studio_render::short_error(&e)));
                    cx.notify();
                }
                this.voice_settled(video, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Speak a sample sentence with the current voice settings.
    pub fn preview_voice(&mut self, cx: &mut Context<Self>) {
        let spec = self.voice_spec();
        let Some(dir) = data_dir().map(|d| d.join("voice-preview")) else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let line = cx
                .background_executor()
                .spawn(async move {
                    studio_render::voice::line("If you double a pizza's radius, do you get twice the pizza? Let's find out.", &spec, &dir)
                })
                .await;
            this.update(cx, |this, cx| match line {
                Ok(l) => {
                    if let Some(a) = &mut this.audio {
                        a.play_full(((usize::MAX, 0), 0), &l.wav);
                    }
                }
                Err(e) => {
                    this.notice = Some(format!("The voice didn't work: {e:#}"));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Mix a scene's narration track in the background, then swap it in. A newer mix for
    /// the scene, or a newer `refresh_voice` of the video, supersedes this one.
    fn mix_voice(&mut self, video: (usize, usize), scene: usize, plan: MixPlan, cx: &mut Context<Self>) {
        let Some(generation) = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)).map(|v| v.voice_gen) else { return };
        self.stage.mix_seq += 1;
        let seq = self.stage.mix_seq;
        self.stage.mix_latest.insert((video, scene), seq);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let MixPlan { wavs, starts, dir } = plan;
            let (wavs2, starts2) = (wavs.clone(), starts.clone());
            let line = cx
                .background_executor()
                .spawn(async move { studio_render::voice::compose(&wavs2, &starts2, &dir).map(|l| (l, dir)) })
                .await;
            let (line, dir) = match line {
                Ok(l) => l,
                Err(e) => {
                    // Don't leave a pending entry behind (it would block pruning) and say why.
                    this.update(cx, |this, cx| {
                        if this.stage.mix_latest.get(&(video, scene)) == Some(&seq) {
                            this.stage.mix_latest.remove(&(video, scene));
                            this.notice = Some(format!("Couldn't mix the narration for chapter {}: {e:#}", scene + 1));
                            cx.notify();
                        }
                    })
                    .ok();
                    return;
                }
            };
            let mut keep: Option<Vec<PathBuf>> = None;
            this.update(cx, |this, cx| {
                if this.stage.mix_latest.get(&(video, scene)) != Some(&seq) {
                    return;
                }
                this.stage.mix_latest.remove(&(video, scene));
                let Some(v) = this.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)) else { return };
                if v.voice_gen != generation {
                    return;
                }
                if let Some(vl) = v.scenes.get_mut(scene).and_then(|s| s.engine.voice.as_mut())
                    // Still the same sentences (the voice may have been re-spoken meanwhile).
                    && vl.parts.iter().map(|p| &p.wav).eq(wavs.iter())
                {
                    vl.wav = line.wav;
                    vl.seconds = line.seconds as f32;
                    vl.starts = starts.iter().map(|s| *s as f32).collect();
                    if this.playing && video == this.current && scene == this.scene {
                        this.sync_audio(true);
                    }
                    cx.notify();
                }
                // Other mixes of this video still in flight will apply tracks this list
                // doesn't know yet: prune only when none are pending.
                if !this.stage.mix_latest.keys().any(|(v, _)| *v == video) {
                    keep = Some(this.projects.iter().flat_map(|p| &p.videos).flat_map(|v| &v.scenes).filter_map(|s| s.engine.voice.as_ref().map(|v| v.wav.clone())).collect());
                }
            })
            .ok();
            // Old mixes pile up as the narration is edited: drop the ones nothing uses
            // (only when the in-use list is known).
            if let Some(keep) = keep {
                cx.background_executor().spawn(async move { prune_tracks(&dir, &keep) }).detach();
            }
        })
        .detach();
    }

    /// Narration is about to be spoken for `video` (it is on, a voice is installed, and some
    /// scene has a line but no voice yet).
    fn voice_pending(&self, video: (usize, usize)) -> bool {
        self.prefs.narration
            && studio_render::kokoro::supported()
            && studio_render::voice::kokoro_ready()
            && self
                .projects
                .get(video.0)
                .and_then(|p| p.videos.get(video.1))
                .is_some_and(|v| v.scenes.iter().any(|s| !s.say.trim().is_empty() && s.engine.voice.is_none()))
    }

    /// Thumbnails for a video that was just opened. Scene lengths depend on narration, and
    /// a voice that arrives re-measures every scene, so when one is about to be spoken the
    /// thumbnails wait for it (`after_voice_change`) instead of running every scene twice.
    pub fn refresh_thumbs_after_voice(&mut self, video: (usize, usize)) {
        if self.voice_pending(video) {
            self.stage.thumbs_deferred.insert(video);
        } else {
            self.refresh_thumbs_for(video);
        }
    }

    /// The voice turned out not to change anything: the deferred thumbnails go ahead now.
    fn flush_deferred_thumbs(&mut self, video: (usize, usize)) {
        if self.stage.thumbs_deferred.remove(&video) {
            self.refresh_thumbs_for(video);
        }
    }

    /// Scene lengths depend on narration: reload the stage and re-measure scenes.
    fn after_voice_change(&mut self, video: (usize, usize), cx: &mut Context<Self>) {
        self.stage.thumbs_deferred.remove(&video);
        if video == self.current {
            self.stage.open = None;
            self.sync_preview();
        }
        self.refresh_thumbs_for(video);
        cx.notify();
    }

    /// Point the stage at the current scene and time: opens the scene if it changed,
    /// otherwise just seeks (seeks are coalesced by the engine).
    pub fn sync_preview(&mut self) {
        let video = self.current;
        let scene = self.scene;
        let Some(req) = self.open_request(video, scene) else {
            if self.stage.open.take().is_some() {
                self.engine.preview.close();
            }
            self.clear_stage_frame();
            self.stage.error = None;
            self.stage.timeline = None;
            return;
        };
        let local = (self.playhead - self.video().scene_start(scene)).max(0.) as f64;
        let key = (video, scene, req.path.clone(), self.video_style(video));
        if self.stage.open.as_ref() != Some(&key) {
            // Playing into the next scene: keep the last frame up and dissolve it out.
            let next_in_line = self.stage.open.as_ref().is_some_and(|(v, s, _, _)| *v == video && s + 1 == scene);
            if self.playing && next_in_line && self.stage.fade_from.is_none() {
                self.stage.fade_from = self.stage.frame.clone().map(|f| (f, std::time::Instant::now()));
            }
            self.stage.generation = self.engine.preview.open(req);
            self.stage.open = Some(key);
            self.stage.loading = true;
            self.stage.timeline = None;
        }
        self.engine.preview.seek(local);
    }

    /// Re-run every scene of the current video for thumbnails, durations and checks.
    pub fn refresh_thumbs(&mut self) {
        self.refresh_thumbs_for(self.current);
    }

    pub fn refresh_thumbs_for(&mut self, video: (usize, usize)) {
        let Some(n) = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)).map(|v| v.scenes.len()) else { return };
        for i in 0..n {
            if let Some(req) = self.open_request(video, i) {
                self.engine.thumbs.request(thumb_key(self.thumb_gen, video, i), req);
            }
        }
    }

    /// Put the playhead on the current scene's key moment (what its thumbnail shows).
    pub fn show_key_moment(&mut self) {
        let start = self.video().scene_start(self.scene);
        let key = self.video().scenes.get(self.scene).and_then(|s| s.engine.key_time).unwrap_or(0.);
        self.playhead = start + key;
        self.at_key_moment = true;
        self.sync_preview();
    }

    /// A scene script's text for the Code view (re-read when the file changes).
    pub fn code_text(&mut self, path: &Path) -> std::rc::Rc<str> {
        let mtime = self.probes.borrow_mut().code_mtime.get(path.to_path_buf(), |p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        if let Some((p, m, text)) = &self.stage.code
            && p == path && *m == mtime {
                return text.clone();
            }
        let text: std::rc::Rc<str> =
            std::fs::read_to_string(path).unwrap_or_else(|e| format!("// could not read {}: {e}", path.display())).into();
        self.stage.code = Some((path.to_path_buf(), mtime, text.clone()));
        text
    }

    /// The style showcase (a short title card) in `style`, installed on first use.
    fn showcase_request(style: usize) -> Option<OpenScene> {
        let dir = data_dir()?.join("showcase");
        let path = studio_render::showcase::install(&dir).ok()?;
        Some(OpenScene {
            path,
            root: dir,
            theme_json: studio_render::style::json(style).to_string(),
            scene: None,
            globals: vec![],
            narration: None,
            narration_lines: vec![],
            narration_cues: vec![],
        })
    }

    /// One still of the style showcase per house style, for the style pickers.
    pub fn request_style_previews(&mut self) {
        for style in 0..studio_render::style::STYLES.len() {
            if let Some(req) = Self::showcase_request(style) {
                self.engine.thumbs.request(STYLE_KEY | style as u64, req);
            }
        }
    }

    /// Point at a house style in a picker (or stop, with `None`): its preview clip plays
    /// in place of the still while the pointer stays, rendered on first hover.
    pub fn hover_style(&mut self, style: Option<usize>, cx: &mut Context<Self>) {
        if self.style_hover.map(|(s, _)| s) == style {
            return;
        }
        self.style_hover = style.map(|s| (s, std::time::Instant::now()));
        if let Some(s) = style
            && !self.style_clips.iter().any(|(k, _)| *k == s)
            && let Some(req) = Self::showcase_request(s)
        {
            self.engine.thumbs.request_clip(STYLE_CLIP_KEY | s as u64, req, STYLE_CLIP_FPS);
        }
        self.ensure_style_ticker(cx);
        cx.notify();
    }

    /// A style card's hover changed.
    pub fn hover_style_card(&mut self, style: usize, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.hover_style(Some(style), cx);
        } else if self.style_hover.is_some_and(|(s, _)| s == style) {
            self.hover_style(None, cx);
        }
    }

    /// Whether the open video shows a live "working" line.
    fn working_visible(&self) -> bool {
        self.projects
            .get(self.current.0)
            .and_then(|p| p.videos.get(self.current.1))
            .is_some_and(|v| v.chat.iter().any(|m| matches!(m, Msg::Working(_))))
    }

    /// Redraws a few times a second while the open video's "working" line is showing: its
    /// clock and stepped animation follow elapsed time, instead of repeating animations
    /// that re-render the whole window every frame.
    pub(crate) fn ensure_working_ticker(&mut self, cx: &mut Context<Self>) {
        if self.working_ticker.is_some() || !self.working_visible() {
            return;
        }
        self.working_ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(WORKING_TICK_MS)).await;
                let keep = this
                    .update(cx, |this, cx| {
                        let on = this.working_visible();
                        if on {
                            cx.notify();
                        } else {
                            this.working_ticker = None;
                        }
                        on
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    /// Redraws at the clip's frame rate while a style is hovered.
    fn ensure_style_ticker(&mut self, cx: &mut Context<Self>) {
        if self.style_ticker.is_some() || self.style_hover.is_none() {
            return;
        }
        self.style_ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs_f64(1.0 / STYLE_CLIP_FPS)).await;
                let keep = this
                    .update(cx, |this, cx| {
                        let on = this.style_hover.is_some();
                        if on {
                            cx.notify();
                        } else {
                            this.style_ticker = None;
                        }
                        on
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    /// The frame of `style`'s clip to show now, when it is hovered and its clip is ready.
    pub fn style_clip_frame(&self, style: usize) -> Option<Still> {
        let (s, since) = self.style_hover?;
        if s != style {
            return None;
        }
        let (_, frames) = self.style_clips.iter().find(|(k, _)| *k == style)?;
        // Hold the last frame for a moment before looping.
        let n = frames.len() + (STYLE_CLIP_FPS as usize);
        let i = (since.elapsed().as_secs_f64() * STYLE_CLIP_FPS) as usize % n.max(1);
        frames.get(i.min(frames.len().saturating_sub(1))).cloned()
    }

    /// The first video backed by scripts (the bundled sample).
    pub fn sample_video(&self) -> Option<(usize, usize)> {
        self.projects.iter().enumerate().find_map(|(pi, p)| p.videos.iter().position(|v| v.has_scripts()).map(|vi| (pi, vi)))
    }

    /// House style changed: everything the engine drew is stale.
    pub fn restyle(&mut self, cx: &mut Context<Self>) {
        self.stage.open = None;
        self.sync_preview();
        self.refresh_thumbs();
        cx.notify();
    }

    fn set_stage_frame(&mut self, image: Option<Arc<RenderImage>>) {
        if let Some(old) = std::mem::replace(&mut self.stage.frame, image) {
            // The dissolving frame is still on screen; it is freed when the dissolve ends.
            if !self.stage.fade_from.as_ref().is_some_and(|(f, _)| Arc::ptr_eq(f, &old)) {
                self.stage.dead.push(old);
            }
        }
    }

    /// Remove the stage frame, queueing its image to be freed.
    pub fn clear_stage_frame(&mut self) {
        if let Some(old) = self.stage.frame.take()
            && !self.stage.fade_from.as_ref().is_some_and(|(f, _)| Arc::ptr_eq(f, &old))
        {
            self.stage.dead.push(old);
        }
    }

    /// Free the images queued in `stage.dead` (call with a context, e.g. while rendering).
    pub fn flush_dead_images(&mut self, window: &mut Window) {
        // `cx.drop_image` can't reach this window: it is taken out of the app while rendering.
        for img in std::mem::take(&mut self.stage.dead) {
            _ = window.drop_image(img);
        }
    }

    /// Queue every thumbnail of `scenes` to be freed (the scenes are going away).
    pub fn retire_scenes(&mut self, scenes: Vec<crate::model::Scene>) {
        self.stage.dead.extend(scenes.into_iter().filter_map(|mut s| s.engine.thumb.take().map(|t| t.0)));
    }

    /// Free the dissolving frame once its dissolve is over.
    pub fn end_fade_if_done(&mut self) {
        if self.stage.fade_from.is_some()
            && self.stage.fade_opacity().is_none()
            && let Some((img, _)) = self.stage.fade_from.take()
            && !self.stage.frame.as_ref().is_some_and(|f| Arc::ptr_eq(f, &img))
        {
            self.stage.dead.push(img);
        }
    }

    fn on_preview(&mut self, ev: PreviewEvent, cx: &mut Context<Self>) {
        let Some((video, scene, _, _)) = self.stage.open.clone() else { return };
        match ev {
            PreviewEvent::Loaded { generation, timeline, elapsed, .. } if generation == self.stage.generation => {
                // Not requested by `sync_preview`: the engine reloaded an edited file.
                let reloaded = !self.stage.loading;
                self.stage.loading = false;
                self.stage.error = None;
                self.stage.load_ms = Some(elapsed.as_millis() as u32);
                let mut mix = None;
                if let Some(s) = self.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)).and_then(|v| v.scenes.get_mut(scene)) {
                    s.engine.duration = Some(timeline.duration as f32);
                    s.engine.key_time = Some(studio_render::preview::thumbnail_time(&timeline) as f32);
                    s.engine.issue = issue_for(&timeline);
                    s.engine.checked = true;
                    mix = plan_voice_mix(s, &timeline);
                }
                if let Some(plan) = mix {
                    self.mix_voice(video, scene, plan, cx);
                }
                // A reload after an edit: the thumbnail is stale too.
                if reloaded
                    && let Some(req) = self.open_request(video, scene) {
                        self.engine.thumbs.request(thumb_key(self.thumb_gen, video, scene), req);
                    }
                self.stage.timeline = Some(timeline);
                if self.at_key_moment && !self.playing && (video, scene) == (self.current, self.scene) {
                    self.show_key_moment();
                }
                cx.notify();
            }
            PreviewEvent::Failed { generation, error, .. } if generation == self.stage.generation => {
                self.stage.loading = false;
                self.stage.timeline = None;
                if let Some(s) = self.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)).and_then(|v| v.scenes.get_mut(scene)) {
                    s.engine.issue = Some(error_issue(&error));
                    s.engine.checked = true;
                }
                self.stage.error = Some(error);
                cx.notify();
            }
            PreviewEvent::Frame { generation, frame, .. } if generation == self.stage.generation => {
                self.set_stage_frame(Some(to_image(frame)));
                // While playing, the ticker redraws every 33 ms and shows this frame then.
                if !self.ticking() {
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn on_thumb(&mut self, ev: ThumbEvent, cx: &mut Context<Self>) {
        let key = ev.key();
        if key & STYLE_CLIP_KEY != 0 {
            let style = (key & !STYLE_CLIP_KEY) as usize;
            if let ThumbEvent::Clip { frames, .. } = ev {
                let frames: Vec<Still> = frames.into_iter().map(|f| Still(to_image(f))).collect();
                self.style_clips.retain(|(k, _)| *k != style);
                self.style_clips.push((style, frames));
                while self.style_clips.len() > STYLE_CLIPS_KEPT {
                    let (_, old) = self.style_clips.remove(0);
                    self.stage.dead.extend(old.into_iter().map(|Still(img)| img));
                }
                cx.notify();
            }
            return;
        }
        if key & STYLE_KEY != 0 {
            let style = (key & !STYLE_KEY) as usize;
            if let (ThumbEvent::Ready { frame, .. }, Some(slot)) = (ev, self.style_previews.get_mut(style)) {
                if let Some(Still(old)) = slot.replace(Still(to_image(frame))) {
                    self.stage.dead.push(old);
                }
                cx.notify();
            }
            return;
        }
        if (key >> 48) & 0xFFF != self.thumb_gen & 0xFFF {
            return; // Asked for before the videos moved.
        }
        let (pi, vi, si) = (((key >> 40) & 0xFF) as usize, ((key >> 20) & 0xFFFFF) as usize, (key & 0xFFFFF) as usize);
        let current = (pi, vi) == self.current;
        let local = self.playhead - self.video().scene_start(self.scene);
        let Some(s) = self.projects.get_mut(pi).and_then(|p| p.videos.get_mut(vi)).and_then(|v| v.scenes.get_mut(si)) else {
            return;
        };
        let mut mix = None;
        let old = match ev {
            ThumbEvent::Ready { frame, timeline, .. } => {
                mix = plan_voice_mix(s, &timeline);
                s.engine.duration = Some(timeline.duration as f32);
                s.engine.key_time = Some(studio_render::preview::thumbnail_time(&timeline) as f32);
                s.engine.issue = issue_for(&timeline);
                s.engine.checked = true;
                s.engine.thumb.replace(Still(to_image(frame)))
            }
            ThumbEvent::Failed { error, .. } => {
                s.engine.issue = Some(error_issue(&error));
                s.engine.checked = true;
                None
            }
            // Scenes never ask for clips.
            ThumbEvent::Clip { .. } => return,
        };
        if let Some(Still(old)) = old {
            self.stage.dead.push(old);
        }
        if let Some(plan) = mix {
            self.mix_voice((pi, vi), si, plan, cx);
        }
        // Real durations replace planned ones: keep the playhead at the same moment of
        // the current scene (or its key moment, the first time it is known).
        if current && !self.playing && !self.scrubbing && self.scene < self.video().scenes.len() {
            let start = self.video().scene_start(self.scene);
            let len = self.video().scenes[self.scene].length();
            if self.at_key_moment {
                self.show_key_moment();
            } else {
                self.playhead = start + local.clamp(0., len);
                self.sync_preview();
            }
        }
        cx.notify();
    }

    /// Queue the current video for a final render.
    pub fn start_render(&mut self, cx: &mut Context<Self>) {
        self.start_render_for(self.current, cx);
    }

    /// Queue `target` (not necessarily the open video) for a final render.
    pub fn start_render_for(&mut self, target: (usize, usize), cx: &mut Context<Self>) {
        // A scene rendered before its narration is spoken comes out silent: speak it
        // first, and render when that finishes (whether or not every line worked).
        if self.voice_pending(target) {
            if self.stage.render_after_voice.insert(target) {
                self.notice = Some("The video saves once the narration is ready.".into());
                self.refresh_voice(target, cx);
            }
            cx.notify();
            return;
        }
        self.queue_render(target, cx);
    }

    /// The narration `refresh_voice` was speaking for `video` is done: start the render
    /// that waited for it.
    fn voice_settled(&mut self, video: (usize, usize), cx: &mut Context<Self>) {
        if self.stage.render_after_voice.remove(&video) {
            self.queue_render(video, cx);
        }
    }

    fn queue_render(&mut self, target: (usize, usize), cx: &mut Context<Self>) {
        let (pi, vi) = target;
        let Some(video) = self.projects.get(pi).and_then(|p| p.videos.get(vi)).cloned() else { return };
        let Some(root) = video.dir.clone() else {
            let Some(chat) = self.chat_of((pi, vi)) else { return };
            chat.push(Msg::Activity {
                icon: IconName::Info,
                ok: false,
                text: "Nothing to save yet: create the video first".into(),
                meta: String::new(),
            });
            cx.notify();
            return;
        };
        if self.jobs.iter().any(|j| j.video == (pi, vi) && j.status.active()) {
            if (pi, vi) == self.current {
                self.screen = crate::app::Screen::Renders;
            }
            cx.notify();
            return;
        }
        let output = OUTPUTS[self.video_output((pi, vi))].1;
        let style = self.video_style((pi, vi));
        let id = self.engine.next_job;
        self.engine.next_job += 1;
        let scenes: Vec<SceneSpec> = video
            .scenes
            .iter()
            .filter_map(|s| {
                s.engine.file.clone().map(|path| SceneSpec {
                    path,
                    title: s.title.clone(),
                    key_time: s.engine.key_time.map(f64::from),
                    expected: s.engine.duration.map(f64::from),
                    narration: s.engine.voice.as_ref().map(|v| studio_render::queue::Narration {
                        wav: v.wav.clone(),
                        seconds: v.seconds as f64,
                        text: studio_render::voice::plain(&v.text),
                        parts: v.parts.iter().map(|p| (p.wav.clone(), p.text.clone())).collect(),
                        cues: v.cues.clone(),
                    }),
                })
            })
            .collect();
        // Another video of this project with the same title would overwrite this one's file.
        let clash = self.projects[pi]
            .videos
            .iter()
            .enumerate()
            .any(|(i, v)| i != vi && file_name_part(&v.title) == file_name_part(&video.title));
        let tag = clash.then(|| root.file_name().map(|f| f.to_string_lossy().into_owned())).flatten();
        // Test and screenshot runs (`STUDIO_HOME`) keep their exports out of the real Videos folder.
        let videos = if crate::store::studio_home().is_some() { None } else { videos_dir() };
        let export = export_path(videos.as_deref(), &root, &self.projects[pi].name, &video.title, tag.as_deref());
        let waiting = self.jobs.iter().filter(|j| j.status.active()).count();
        self.jobs.insert(
            0,
            RenderJob {
                id,
                title: video.title.clone(),
                project: self.projects[pi].name.to_string(),
                output: output.label(),
                status: if waiting == 0 { JobStatus::Rendering { scene: 1, of: scenes.len() } } else { JobStatus::Waiting(waiting) },
                thumb: video.cover_still(),
                time: "—".into(),
                progress: 0.,
                video: (pi, vi),
                path: None,
                cached: 0,
                rendered: 0,
            },
        );
        self.engine.queue.push(VideoRender {
            id,
            root,
            scenes,
            style_json: studio_render::style::json(style).to_string(),
            output,
            backend: BACKENDS[self.prefs.renderer.min(BACKENDS.len() - 1)].1,
            encoder: ENCODERS[self.prefs.encoder.min(ENCODERS.len() - 1)].1,
            export,
            fonts_dir: self.engine.fonts_dir.clone(),
            captions: match self.prefs.captions {
                0 => studio_render::queue::Captions::Off,
                2 => studio_render::queue::Captions::Burned,
                _ => studio_render::queue::Captions::Srt,
            },
            watermark: self.watermark_for(pi),
            title: video.title.clone(),
            slides: slide_formats(self.video_slides((pi, vi))),
        });
        self.projects[pi].videos[vi].status = VideoStatus::Building;
        if let Some(chat) = self.chat_of((pi, vi)) {
            chat.retain(|m| !matches!(m, Msg::Rendering));
            chat.push(Msg::Rendering);
        }
        if (pi, vi) == self.current {
            self.render_progress = Some(0.);
            self.chat_scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    pub fn cancel_job(&mut self, id: u64, cx: &mut Context<Self>) {
        self.engine.queue.cancel(id);
        cx.notify();
    }

    pub fn set_queue_paused(&mut self, paused: bool, cx: &mut Context<Self>) {
        self.queue_paused = paused;
        self.engine.queue.set_paused(paused);
        cx.notify();
    }

    fn renumber_waiting(&mut self) {
        let mut n = 0;
        for j in self.jobs.iter_mut().rev() {
            if let JobStatus::Waiting(_) = j.status {
                n += 1;
                j.status = JobStatus::Waiting(n);
            }
        }
    }

    fn on_queue(&mut self, ev: QueueEvent, cx: &mut Context<Self>) {
        let id = ev.id();
        let Some(ix) = self.jobs.iter().position(|j| j.id == id) else { return };
        let video = self.jobs[ix].video;
        let mine = video == self.current;
        match ev {
            QueueEvent::Started { .. } => {
                self.jobs[ix].status = JobStatus::Rendering { scene: 1, of: self.projects[video.0].videos[video.1].scenes.len() };
                self.renumber_waiting();
            }
            QueueEvent::Scene { index, of, cached, .. } => {
                self.jobs[ix].status = JobStatus::Rendering { scene: index + 1, of };
                if cached {
                    self.jobs[ix].cached += 1;
                }
            }
            QueueEvent::Engine { renderer, output, .. } => {
                self.engine.backend_info = Some(format!("{renderer} · {output}"));
            }
            QueueEvent::Progress { fraction, .. } => {
                self.jobs[ix].progress = fraction;
                if mine {
                    self.render_progress = Some(fraction);
                }
            }
            QueueEvent::Problem { .. } => {}
            QueueEvent::Exporting { .. } => {
                self.jobs[ix].status = JobStatus::Exporting;
                self.jobs[ix].progress = 1.;
            }
            QueueEvent::Done { path, elapsed, duration, rendered, cached, slides, slides_problem, .. } => {
                let place = saved_place(&path);
                let job = &mut self.jobs[ix];
                job.status = JobStatus::Done(crate::model::format_short(duration.round() as u32));
                job.time = format_elapsed(elapsed);
                job.path = Some(path);
                job.progress = 1.;
                job.rendered = rendered;
                job.cached = cached;
                self.projects[video.0].videos[video.1].status = VideoStatus::Ready;
                if mine {
                    self.render_progress = None;
                }
                let meta = format!("{:.1} s", elapsed.as_secs_f32());
                let (text, notice) = match (&slides_problem, slides.is_empty()) {
                    (Some(why), _) => {
                        let short = studio_render::short_error(why);
                        (
                            format!("Video saved to {place}. The lecture slides couldn't be made: {short}"),
                            format!("Video saved to {place}, but the lecture slides couldn't be made: {short}"),
                        )
                    }
                    (None, false) => (
                        format!("Video and lecture slides saved to {place}"),
                        format!("Video and lecture slides saved to {place}. Open Saved videos to find them."),
                    ),
                    (None, true) => (format!("Video saved to {place}"), format!("Video saved to {place}. Open Saved videos to find it.")),
                };
                self.notice = Some(notice);
                self.replace_rendering_row(video, Msg::Activity { icon: IconName::Check, ok: true, text, meta });
            }
            QueueEvent::Failed { scene, error, .. } => {
                let short = studio_render::short_error(&error);
                let where_ = scene.map(|s| format!("Chapter {}: ", s + 1)).unwrap_or_default();
                self.jobs[ix].status = JobStatus::Failed(format!("{where_}{short}"));
                self.projects[video.0].videos[video.1].status = VideoStatus::Failed;
                if let (Some(s), true) = (scene, mine)
                    && let Some(sc) = self.projects[video.0].videos[video.1].scenes.get_mut(s) {
                        sc.engine.issue = Some(error_issue(&error));
                    }
                if mine {
                    self.render_progress = None;
                }
                self.replace_rendering_row(video, Msg::Activity {
                    icon: IconName::X,
                    ok: false,
                    text: format!("Couldn't save the video · {where_}{short}"),
                    meta: String::new(),
                });
            }
            QueueEvent::Cancelled { .. } => {
                self.jobs[ix].status = JobStatus::Cancelled;
                let v = &mut self.projects[video.0].videos[video.1];
                if v.status == VideoStatus::Building {
                    v.status = VideoStatus::Ready;
                }
                if let Some(chat) = self.chat_of(video) {
                    chat.retain(|m| !matches!(m, Msg::Rendering));
                }
                if mine {
                    self.render_progress = None;
                }
                self.renumber_waiting();
            }
        }
        cx.notify();
    }

    fn replace_rendering_row(&mut self, video: (usize, usize), msg: Msg) {
        if let Some(chat) = self.chat_of(video) {
            match chat.iter().position(|m| matches!(m, Msg::Rendering)) {
                Some(pos) => chat[pos] = msg,
                None => chat.push(msg),
            }
        }
        self.persist(video);
        self.chat_scroll.scroll_to_bottom();
    }
}

#[cfg(test)]
mod export_path_tests {
    use super::export_path;
    use std::path::Path;

    #[test]
    fn saved_videos_go_where_a_teacher_looks() {
        let videos = Path::new("V");
        let root = Path::new("R/video");
        assert_eq!(export_path(Some(videos), root, "Class 7", "Fractions: halves?", None), Path::new("V/Class 7/Fractions halves.mp4"));
        // Two videos of one project with one title don't overwrite each other.
        assert_eq!(export_path(Some(videos), root, "Class 7", "Quiz", Some("quiz-2")), Path::new("V/Class 7/Quiz (quiz-2).mp4"));
        // Names Windows would refuse or mangle still make a file.
        assert_eq!(export_path(Some(videos), root, "...", "*?", None), Path::new("V/Video.mp4"));
        assert_eq!(export_path(None, root, "Class 7", "Quiz.", None), Path::new("R/video/exports/Quiz.mp4"));
    }
}
