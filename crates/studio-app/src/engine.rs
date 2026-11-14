//! Glue between the workspace and `studio-render`: the live stage preview, scene
//! thumbnails, and the render queue.

use crate::app::Workspace;
use crate::model::{Issue, JobStatus, Msg, RenderJob, Still, VideoStatus, VoiceLine, VoicePart};
use gpui::{AsyncApp, Context, RenderImage, WeakEntity, prelude::FluentBuilder};
use smallvec::smallvec;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// Lay a scene's narration sentences out where its timeline placed them (`narrate(n)`
/// marks), mixing a new track when that moved anything. True when the track changed.
fn layout_voice(s: &mut crate::model::Scene, t: &TimelineInfo) -> bool {
    let Some(v) = s.engine.voice.as_mut() else { return false };
    if v.parts.is_empty() {
        return false;
    }
    let lines = v.lines();
    let starts = if studio_render::preview::uses_narrate(t) {
        studio_render::preview::narration_starts(t, &lines)
    } else {
        studio_render::voice::sequential_starts(&lines)
    };
    let same = starts.len() == v.starts.len() && starts.iter().zip(&v.starts).all(|(a, b)| (*a as f32 - b).abs() < 0.01);
    let Some(dir) = v.wav.parent().map(Path::to_path_buf) else { return false };
    if same {
        return false;
    }
    let wavs: Vec<PathBuf> = v.parts.iter().map(|p| p.wav.clone()).collect();
    match studio_render::voice::compose(&wavs, &starts, &dir) {
        Ok(line) => {
            v.wav = line.wav;
            v.seconds = line.seconds as f32;
            v.starts = starts.iter().map(|s| *s as f32).collect();
            true
        }
        Err(_) => false,
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
    pub code: Option<(PathBuf, Option<std::time::SystemTime>, String)>,
    /// The previous scene's last frame, dissolving out over the next scene during
    /// playback (the same dissolve the export uses), and when it started.
    pub fade_from: Option<(Arc<RenderImage>, std::time::Instant)>,
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
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
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
        })
    }

    pub fn voice_quality(&self) -> studio_render::kokoro::Quality {
        use studio_render::kokoro::Quality;
        Quality::ALL[self.prefs.voice_model.min(Quality::ALL.len() - 1)]
    }

    /// The picked natural-voice model is downloaded.
    pub fn voice_quality_installed(&self) -> bool {
        studio_render::voice::kokoro_dir().is_some_and(|d| studio_render::kokoro::installed(d, self.voice_quality()))
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
        cx.notify();
        let (tx, rx) = async_channel::unbounded::<f32>();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let task = cx.background_executor().spawn(async move {
                let mut last = 0.0f32;
                studio_render::kokoro::espeak::install(&dir, move |done, total| {
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
                match result {
                    Ok(()) => {
                        this.espeak = NaturalVoice::Ready;
                        studio_render::voice::reset_kokoro();
                        this.refresh_voice(this.current, cx);
                    }
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
        let Some(speaker) = self.voice_spec().speaker() else {
            return "Natural voice not downloaded".into();
        };
        let id = speaker.voice;
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
        cx.notify();
        let (tx, rx) = async_channel::unbounded::<f32>();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let task = cx.background_executor().spawn(async move {
                let mut last = 0.0f32;
                studio_render::tools::download_ffmpeg(move |done, total| {
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
                match result {
                    Ok(()) => {
                        this.ffmpeg_dl = NaturalVoice::Ready;
                        this.engine.ffmpeg = true;
                    }
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
        let Some(dir) = studio_render::voice::kokoro_dir().map(Path::to_path_buf) else { return };
        if matches!(self.natural, NaturalVoice::Downloading(_)) {
            return;
        }
        self.natural = NaturalVoice::Downloading(0.);
        let quality = self.voice_quality();
        cx.notify();
        let (tx, rx) = async_channel::unbounded::<f32>();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let task = cx.background_executor().spawn(async move {
                let mut last = 0.0f32;
                studio_render::kokoro::download(&dir, quality, move |done, total| {
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
        let Some(dir) = v.dir.clone() else { return };
        let enabled = self.prefs.narration;
        let lines: Vec<(usize, String)> =
            v.scenes.iter().enumerate().map(|(i, s)| (i, s.say.trim().to_string())).collect();
        if !enabled {
            let mut changed = false;
            if let Some(v) = self.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)) {
                for s in &mut v.scenes {
                    changed |= s.engine.voice.take().is_some();
                }
            }
            if changed {
                self.after_voice_change(video, cx);
            }
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
                                return (i, text, None);
                            }
                            // Each sentence on its own, so the scene can place it.
                            let spoken: Option<Vec<VoicePart>> = studio_render::voice::sentences(&text)
                                .into_iter()
                                .map(|s| {
                                    studio_render::voice::line(&s, &spec, &cache)
                                        .ok()
                                        .map(|l| VoicePart { wav: l.wav, seconds: l.seconds as f32, text: s })
                                })
                                .collect();
                            let line = spoken.filter(|p| !p.is_empty()).and_then(|parts| {
                                let lines: Vec<f64> = parts.iter().map(|p| p.seconds as f64).collect();
                                let starts = studio_render::voice::sequential_starts(&lines);
                                let wavs: Vec<PathBuf> = parts.iter().map(|p| p.wav.clone()).collect();
                                let track = studio_render::voice::compose(&wavs, &starts, &cache).ok()?;
                                Some((track, parts, starts))
                            });
                            (i, text, line)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                let mut changed = false;
                if let Some(v) = this.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)) {
                    for (i, text, line) in spoken {
                        let Some(s) = v.scenes.get_mut(i) else { continue };
                        if s.say.trim() != text {
                            continue;
                        }
                        let new = line.map(|(l, parts, starts)| VoiceLine {
                            wav: l.wav,
                            seconds: l.seconds as f32,
                            text,
                            parts,
                            starts: starts.iter().map(|s| *s as f32).collect(),
                        });
                        if s.engine.voice != new {
                            s.engine.voice = new;
                            changed = true;
                        }
                    }
                }
                if changed {
                    this.after_voice_change(video, cx);
                }
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

    /// Scene lengths depend on narration: reload the stage and re-measure scenes.
    fn after_voice_change(&mut self, video: (usize, usize), cx: &mut Context<Self>) {
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
            self.stage.frame = None;
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
    pub fn code_text(&mut self, path: &Path) -> String {
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some((p, m, text)) = &self.stage.code
            && p == path && *m == mtime {
                return text.clone();
            }
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| format!("// could not read {}: {e}", path.display()));
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

    fn set_stage_frame(&mut self, image: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        if let Some(old) = std::mem::replace(&mut self.stage.frame, image) {
            // The dissolving frame is still on screen; it is freed when the dissolve ends.
            if !self.stage.fade_from.as_ref().is_some_and(|(f, _)| Arc::ptr_eq(f, &old)) {
                cx.drop_image(old, None);
            }
        }
    }

    /// Free the dissolving frame once its dissolve is over.
    pub fn end_fade_if_done(&mut self, cx: &mut Context<Self>) {
        if self.stage.fade_from.is_some()
            && self.stage.fade_opacity().is_none()
            && let Some((img, _)) = self.stage.fade_from.take()
            && !self.stage.frame.as_ref().is_some_and(|f| Arc::ptr_eq(f, &img))
        {
            cx.drop_image(img, None);
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
                if let Some(s) = self.projects.get_mut(video.0).and_then(|p| p.videos.get_mut(video.1)).and_then(|v| v.scenes.get_mut(scene)) {
                    s.engine.duration = Some(timeline.duration as f32);
                    s.engine.key_time = Some(studio_render::preview::thumbnail_time(&timeline) as f32);
                    s.engine.issue = issue_for(&timeline);
                    s.engine.checked = true;
                    if layout_voice(s, &timeline) && self.playing {
                        self.sync_audio(true);
                    }
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
                self.set_stage_frame(Some(to_image(frame)), cx);
                cx.notify();
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
                    for Still(img) in old {
                        cx.drop_image(img, None);
                    }
                }
                cx.notify();
            }
            return;
        }
        if key & STYLE_KEY != 0 {
            let style = (key & !STYLE_KEY) as usize;
            if let (ThumbEvent::Ready { frame, .. }, Some(slot)) = (ev, self.style_previews.get_mut(style)) {
                if let Some(Still(old)) = slot.replace(Still(to_image(frame))) {
                    cx.drop_image(old, None);
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
        let old = match ev {
            ThumbEvent::Ready { frame, timeline, .. } => {
                layout_voice(s, &timeline);
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
            cx.drop_image(old, None);
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
        let (pi, vi) = self.current;
        let video = self.video().clone();
        let Some(root) = video.dir.clone() else {
            let Some(chat) = self.chat_of((pi, vi)) else { return };
            chat.push(Msg::Activity {
                icon: IconName::Info,
                ok: false,
                text: "Nothing to render yet: build the video first".into(),
                meta: String::new(),
            });
            cx.notify();
            return;
        };
        if self.jobs.iter().any(|j| j.video == (pi, vi) && j.status.active()) {
            self.screen = crate::app::Screen::Renders;
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
                    expected: s.engine.duration.map(f64::from),
                    narration: s.engine.voice.as_ref().map(|v| studio_render::queue::Narration {
                        wav: v.wav.clone(),
                        seconds: v.seconds as f64,
                        text: v.text.clone(),
                        parts: v.parts.iter().map(|p| (p.wav.clone(), p.text.clone())).collect(),
                    }),
                })
            })
            .collect();
        // Keep the title readable; drop only what file systems reject.
        let safe: String = video
            .title
            .chars()
            .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let export = root.join("exports").join(format!("{} {}x{} {}fps.mp4", safe.trim(), output.width, output.height, output.fps));
        let waiting = self.jobs.iter().filter(|j| j.status.active()).count();
        self.jobs.insert(
            0,
            RenderJob {
                id,
                title: video.title.clone(),
                project: self.projects[pi].name.to_string(),
                output: output.label(),
                status: if waiting == 0 { JobStatus::Rendering { scene: 1, of: scenes.len() } } else { JobStatus::Waiting(waiting) },
                art: video.cover(),
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
            QueueEvent::Done { path, elapsed, duration, rendered, cached, .. } => {
                let job = &mut self.jobs[ix];
                job.status = JobStatus::Done(crate::model::format_short(duration.round() as u32));
                job.time = format_elapsed(elapsed);
                job.path = Some(path);
                job.progress = 1.;
                job.rendered = rendered;
                job.cached = cached;
                let label = job.output.clone();
                self.projects[video.0].videos[video.1].status = VideoStatus::Ready;
                if mine {
                    self.render_progress = None;
                }
                let meta = format!("{:.1} s", elapsed.as_secs_f32());
                let text = if cached > 0 {
                    format!("Rendered {label} · {rendered} new, {cached} cached")
                } else {
                    format!("Rendered {label}")
                };
                self.replace_rendering_row(video, Msg::Activity { icon: IconName::Check, ok: true, text, meta });
            }
            QueueEvent::Failed { scene, error, .. } => {
                let short = studio_render::short_error(&error);
                let where_ = scene.map(|s| format!("Scene {}: ", s + 1)).unwrap_or_default();
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
                    text: format!("Render failed · {where_}{short}"),
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
