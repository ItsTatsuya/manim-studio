//! The workspace: one root entity that owns navigation and app state (projects, engine,
//! assistant turns, voice). Screens live in `views/*` as `impl Workspace` blocks.

use studio_ui::motion;
use crate::engine::{Engine, Stage};
use crate::model::{self, JobStatus, Msg, Project, Provider, ProviderStatus, RenderJob, VideoStatus};
use crate::prefs::Prefs;
use gpui::{
    AnimationExt, AnyElement, App, AppContext, Bounds, StyledImage, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, ScrollHandle, SharedString,
    Styled, Subscription, Task, TitlebarOptions, Window,
    WindowBounds, WindowControlArea, WindowDecorations, WindowOptions, div, point,
    prelude::FluentBuilder, px, size,
};
use std::time::{Duration, Instant};
use studio_ui::components::{
    Button, Chip, Dot, Segment, Segmented, Submitted, TextInput, Tone, floating,
};
use studio_ui::theme::{self, ActiveTheme, FONT_UI, num_features, radius, space, text, ui_features};
use studio_ui::{Icon, IconName};

use crate::shortcuts::*;

pub fn init(cx: &mut App) {
    crate::shortcuts::bind(cx);
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

pub fn open_main_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Manim Studio".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(15.))),
        }),
        window_min_size: Some(size(px(960.), px(620.))),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("app.manimstudio.Studio".into()),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        let mut prefs = Prefs::load();
        // `STUDIO_PALETTE=frost` / `STUDIO_THEME=light|dark`: a look for screenshots.
        if let Some(p) = std::env::var("STUDIO_PALETTE").ok().and_then(|v| theme::Palette::ALL.into_iter().find(|p| p.label().eq_ignore_ascii_case(&v))) {
            prefs.palette = p;
        }
        match std::env::var("STUDIO_THEME").as_deref() {
            Ok("light") => prefs.theme = theme::ThemeMode::Light,
            Ok("dark") => prefs.theme = theme::ThemeMode::Dark,
            _ => {}
        }
        theme::apply(prefs.theme, prefs.palette, window, cx);
        cx.new(|cx| Workspace::new(prefs, window, cx))
    })
    .expect("failed to open the Manim Studio window");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Onboarding,
    Home,
    Video,
    Renders,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Storyboard,
    Studio,
    Code,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightTab {
    Assistant,
    Narration,
    Assets,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Assistants,
    Styles,
    Voice,
    Rendering,
    Shortcuts,
    Updates,
    About,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 8] = [
        SettingsPage::General,
        SettingsPage::Assistants,
        SettingsPage::Styles,
        SettingsPage::Voice,
        SettingsPage::Rendering,
        SettingsPage::Shortcuts,
        SettingsPage::Updates,
        SettingsPage::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Assistants => "Assistants & usage",
            SettingsPage::Styles => "House styles",
            SettingsPage::Voice => "Voice & captions",
            SettingsPage::Rendering => "Rendering",
            SettingsPage::Shortcuts => "Keyboard shortcuts",
            SettingsPage::Updates => "Updates",
            SettingsPage::About => "About",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            SettingsPage::General => IconName::Gear,
            SettingsPage::Assistants => IconName::Key,
            SettingsPage::Styles => IconName::Palette,
            SettingsPage::Voice => IconName::Mic,
            SettingsPage::Rendering => IconName::Film,
            SettingsPage::Shortcuts => IconName::Keyboard,
            SettingsPage::Updates => IconName::Download,
            SettingsPage::About => IconName::Info,
        }
    }
}

pub struct Workspace {
    pub prefs: Prefs,
    pub screen: Screen,
    pub onboarding_step: usize,
    pub providers: Vec<ProviderStatus>,
    pub default_provider: Provider,
    pub projects: Vec<Project>,
    pub current: (usize, usize),
    pub mode: Mode,
    pub scene: usize,
    pub right_tab: RightTab,
    pub playing: bool,
    pub playhead: f32,
    pub render_progress: Option<f32>,
    pub jobs: Vec<RenderJob>,
    pub queue_filter: usize,
    pub queue_paused: bool,
    pub settings_page: SettingsPage,
    pub open_menu: Option<&'static str>,
    /// The menu last closed by a click outside it, and when. Pressing a
    /// dropdown's own trigger closes it on mouse-down; this stops the click
    /// that follows from reopening it.
    menu_dismissed: Option<(&'static str, Instant)>,
    pub toast: bool,
    /// The sidebar row whose menu is open, and the one last closed by a click outside.
    pub row_menu: Option<crate::manage::RowTarget>,
    pub row_menu_dismissed: Option<(crate::manage::RowTarget, Instant)>,
    /// The sidebar row being renamed inline.
    pub rename: Option<crate::manage::RowTarget>,
    pub rename_input: Entity<TextInput>,
    /// A delete waiting for confirmation.
    pub confirm: Option<crate::manage::Confirm>,
    /// The folder the Home composer files new videos into (else the open video's).
    pub home_project: Option<usize>,
    /// A one-line message for the teacher (errors, "connect an assistant first").
    pub notice: Option<String>,
    /// Documents attached in the Home composer.
    pub attachments: Vec<std::path::PathBuf>,
    /// Assistant turns in flight.
    pub running: Vec<crate::assistant::Running>,
    pub home_input: Entity<TextInput>,
    /// The "type a length" box in the new-video options.
    pub length_input: Entity<TextInput>,
    /// Where the new-video Options button sat last frame (to open its panel up or down).
    pub options_anchor: std::rc::Rc<std::cell::Cell<Bounds<gpui::Pixels>>>,
    /// The look being hovered in the new-video options (shows its preview).
    pub look_hover: Option<usize>,
    pub chat_input: Entity<TextInput>,
    pub revise_input: Entity<TextInput>,
    pub watermark_input: Entity<TextInput>,
    pub search_input: Entity<TextInput>,
    /// Command palette (Ctrl K): open, selected row, query.
    pub palette: bool,
    pub palette_sel: usize,
    pub palette_input: Entity<TextInput>,
    /// Editing one narration line in the Narration tab: (scene, input).
    pub narration_input: Entity<TextInput>,
    pub editing_line: Option<usize>,
    pub chat_scroll: ScrollHandle,
    /// Bounds of the stage's scrub bar (captured at paint) and whether it is being dragged.
    pub scrub_bounds: std::rc::Rc<std::cell::Cell<Bounds<gpui::Pixels>>>,
    pub scrubbing: bool,
    pub volume_bounds: std::rc::Rc<std::cell::Cell<Bounds<gpui::Pixels>>>,
    pub volume_dragging: bool,
    /// The playhead sits on the selected scene's key moment (not a place the user
    /// scrubbed or played to), so Play starts the scene from its beginning.
    pub at_key_moment: bool,
    /// The sample lesson rendered in each house style (style pickers).
    pub style_previews: Vec<Option<model::Still>>,
    /// Preview clips of recently hovered house styles: (style, frames).
    pub style_clips: Vec<(usize, Vec<model::Still>)>,
    /// The house style under the pointer in a picker, and since when.
    pub style_hover: Option<(usize, Instant)>,
    pub(crate) style_ticker: Option<Task<()>>,
    /// Bumped when videos move, so thumbnails asked for at old positions are dropped.
    pub thumb_gen: u64,
    pub engine: Engine,
    pub stage: Stage,
    /// Narration playback (absent without an audio device).
    pub audio: Option<crate::audio::Audio>,
    pub natural: crate::engine::NaturalVoice,
    /// Stage overlays and playback: safe-area guides, narration captions, loop the scene.
    pub stage_safe: bool,
    /// Watermark sliders in Settings: hit boxes, and the one being dragged.
    pub wm_bounds: [std::rc::Rc<std::cell::Cell<Bounds<gpui::Pixels>>>; 3],
    pub wm_drag: Option<usize>,
    pub stage_captions: bool,
    pub stage_loop: bool,
    /// espeak-ng, the helper for words the dictionaries don't know.
    pub espeak: crate::engine::NaturalVoice,
    /// ffmpeg's first-run download (Ready when it was already on this computer).
    pub ffmpeg_dl: crate::engine::NaturalVoice,
    pub update: crate::update::UpdateState,
    focus: FocusHandle,
    ticker: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl Workspace {
    fn new(prefs: Prefs, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home_input = cx.new(|cx| {
            TextInput::new(
                "Describe a lesson, drop in notes, or paste a full script…",
                window,
                cx,
            )
            .font_size(px(15.), px(24.))
            .lines(2, 10)
        });
        let chat_input = cx.new(|cx| {
            TextInput::new("Ask for a change. Use @ to pick a scene.", window, cx).lines(2, 8)
        });
        let revise_input = cx.new(|cx| {
            TextInput::new(
                "Tell the assistant what to change, e.g. “merge scenes 1 and 2”. Revisions only rewrite the plan.",
                window,
                cx,
            )
            .lines(1, 6)
        });
        let search_input = cx.new(|cx| TextInput::new("Search", window, cx).lines(1, 1));
        let palette_input = cx.new(|cx| TextInput::new("Type a command or a video name", window, cx).lines(1, 1));
        let rename_input = cx.new(|cx| TextInput::new("Name", window, cx).lines(1, 1));
        let length_input = cx.new(|cx| TextInput::new("e.g. 3:30", window, cx).lines(1, 1));
        let narration_input = cx.new(|cx| TextInput::new("What the narrator says in this scene", window, cx).lines(2, 8));
        let watermark_input = cx.new(|cx| {
            let mut i = TextInput::new("e.g. Ms. Rao · Grade 8 Science", window, cx).lines(1, 1);
            i.set_text(prefs.watermark.text.clone(), cx);
            i
        });
        let subs = vec![
            cx.observe(&search_input, |_, _, cx| cx.notify()),
            // Typing refilters the palette from the top.
            cx.observe(&palette_input, |this, _, cx| {
                this.palette_sel = 0;
                cx.notify();
            }),
            cx.subscribe_in(&palette_input, window, |this, _, _: &Submitted, window, cx| this.palette_run(None, window, cx)),
            // Send buttons enable as soon as there is something to send.
            cx.observe(&home_input, |_, _, cx| cx.notify()),
            cx.observe(&chat_input, |_, _, cx| cx.notify()),
            cx.observe(&revise_input, |_, _, cx| cx.notify()),
            // The watermark text is saved as it is typed.
            cx.observe(&watermark_input, |this, input, cx| {
                let text = input.read(cx).text().to_string();
                if text != this.prefs.watermark.text {
                    this.prefs.watermark.text = text;
                    this.save_prefs();
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                theme::apply(this.prefs.theme, this.prefs.palette, window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&home_input, window, |this, _, Submitted(text), _, cx| {
                this.start_plan(text.clone(), cx)
            }),
            cx.subscribe_in(&chat_input, window, |this, input, Submitted(text), _, cx| {
                let text = text.clone();
                input.update(cx, |i, cx| i.clear(cx));
                this.send_chat(text, cx)
            }),
            cx.subscribe_in(&rename_input, window, |this, _, _: &Submitted, _, cx| this.commit_rename(cx)),
            cx.subscribe_in(&length_input, window, |this, _, _: &Submitted, _, cx| this.commit_length(cx)),
            cx.subscribe_in(&length_input, window, |this, _, _: &studio_ui::components::Blurred, _, cx| this.commit_length(cx)),
            cx.subscribe_in(&rename_input, window, |this, _, _: &studio_ui::components::Blurred, _, cx| this.commit_rename(cx)),
            cx.subscribe_in(&narration_input, window, |this, _, Submitted(text), _, cx| {
                let text = text.clone();
                this.save_narration_line(text, cx)
            }),
            cx.subscribe_in(&revise_input, window, |this, input, Submitted(text), _, cx| {
                let text = text.clone();
                input.update(cx, |i, cx| i.clear(cx));
                this.revise_storyboard(text, cx)
            }),
        ];
        let screen = if prefs.onboarded { Screen::Video } else { Screen::Onboarding };
        let start_volume = if prefs.muted { 0. } else { prefs.volume.clamp(0., 1.) };
        let mut projects = crate::store::load();
        crate::store::apply_folder_order(&mut projects, &prefs.folder_order);
        let engine = Engine::start();
        let last = prefs.last_video.clone();
        let last_found = projects
            .iter()
            .enumerate()
            .find_map(|(pi, p)| p.videos.iter().position(|v| last.is_some() && v.dir == last).map(|vi| (pi, vi)));
        let current = last_found
            .or_else(|| {
                projects.iter().enumerate().find_map(|(pi, p)| p.videos.iter().position(|v| v.has_scripts()).map(|vi| (pi, vi)))
            })
            .unwrap_or((0, 0));
        // Reopen the last video; with none (first run after "Start my own", or it was
        // deleted) open on the composer rather than the sample lesson.
        let screen = if screen == Screen::Video && last_found.is_none() { Screen::Home } else { screen };
        let providers = [Provider::Claude, Provider::OpenAi]
            .map(|p| {
                let mut s = ProviderStatus::unknown(p);
                if let Some(u) = prefs.usage_for(p) {
                    s.window_used = u.window;
                    s.week_used = u.week;
                    s.resets_at = u.resets_at;
                    s.week_resets_at = u.week_resets_at;
                }
                s
            })
            .to_vec();
        let default_provider = Provider::from_key(&prefs.default_provider).unwrap_or(Provider::Claude);
        let mut this = Self {
            prefs,
            screen,
            onboarding_step: 0,
            providers,
            default_provider,
            projects,
            current,
            mode: Mode::Studio,
            scene: 0,
            right_tab: RightTab::Assistant,
            playing: false,
            playhead: 0.,
            render_progress: None,
            jobs: Vec::new(),
            queue_filter: 0,
            queue_paused: false,
            settings_page: SettingsPage::Assistants,
            open_menu: None,
            menu_dismissed: None,
            toast: false,
            row_menu: None,
            row_menu_dismissed: None,
            rename: None,
            rename_input,
            confirm: None,
            home_project: None,
            notice: None,
            attachments: Vec::new(),
            running: Vec::new(),
            home_input,
            length_input,
            options_anchor: Default::default(),
            look_hover: None,
            chat_input,
            revise_input,
            watermark_input,
            search_input,
            palette: false,
            palette_sel: 0,
            palette_input,
            narration_input,
            editing_line: None,
            chat_scroll: ScrollHandle::new(),
            scrub_bounds: Default::default(),
            scrubbing: false,
            volume_bounds: Default::default(),
            volume_dragging: false,
            at_key_moment: true,
            style_previews: vec![None; studio_render::style::STYLES.len()],
            style_clips: Vec::new(),
            style_hover: None,
            style_ticker: None,
            thumb_gen: 0,
            engine,
            stage: Stage::default(),
            audio: crate::audio::Audio::new().map(|mut a| {
                a.set_volume(start_volume);
                a
            }),
            natural: crate::engine::NaturalVoice::Missing,
            stage_safe: false,
            wm_bounds: Default::default(),
            wm_drag: None,
            stage_captions: false,
            stage_loop: false,
            espeak: crate::engine::NaturalVoice::Missing,
            ffmpeg_dl: crate::engine::NaturalVoice::Missing,
            update: crate::update::UpdateState::Idle,
            focus: cx.focus_handle(),
            ticker: None,
            _subs: subs,
        };
        // Give the window a focused element so global shortcuts dispatch
        // before the user clicks anything.
        window.focus(&this.focus, cx);
        // ffmpeg isn't in the installer: fetch it in the background on first launch.
        if this.engine.ffmpeg {
            this.ffmpeg_dl = crate::engine::NaturalVoice::Ready;
        } else if cfg!(windows) {
            this.download_ffmpeg(cx);
        }
        this.natural = crate::engine::NaturalVoice::detect(this.voice_quality());
        // A voice downloaded before ONNX Runtime moved out of the program only lacks the
        // runtime DLL: top it up quietly (~6 MB) instead of asking for a download.
        if studio_render::voice::kokoro_dir().is_some_and(|d| studio_render::kokoro::needs_runtime_only(d, this.voice_quality())) {
            this.download_natural_voice(cx);
        }
        if studio_render::voice::espeak().is_some() {
            this.espeak = crate::engine::NaturalVoice::Ready;
        }
        this.apply_dev_overrides();
        this.playhead = this.video().scene_start(this.scene);
        this.listen_to_engine(cx);
        this.sync_preview();
        this.refresh_thumbs();
        this.request_style_previews();
        this.detect_providers(cx);
        this.refresh_voice(this.current, cx);
        this.schedule_update_check(cx);
        if this.screen == Screen::Onboarding {
            this.start_onboarding_demo(cx);
        } else if this.prefs.narration && this.natural == crate::engine::NaturalVoice::Missing {
            // Set up before narration became natural-voice only.
            this.notice = Some("Narration needs the natural voice. Download it once in Settings → Voice & captions.".into());
        }
        this
    }

    /// `STUDIO_SCREEN=home|studio|storyboard|code|renders|settings|settings-<page>|onboarding`
    /// opens a screen directly, for screenshots and visual checks.
    fn apply_dev_overrides(&mut self) {
        let Ok(screen) = std::env::var("STUDIO_SCREEN") else { return };
        match screen.as_str() {
            "onboarding" => self.screen = Screen::Onboarding,
            "home" => self.screen = Screen::Home,
            "renders" => self.screen = Screen::Renders,
            "settings" | "settings-general" | "settings-voice" | "settings-rendering" | "settings-styles"
            | "settings-shortcuts" | "settings-updates" => {
                self.screen = Screen::Settings;
                match screen.as_str() {
                    "settings-updates" => self.settings_page = SettingsPage::Updates,
                    "settings-voice" => self.settings_page = SettingsPage::Voice,
                    "settings-rendering" => self.settings_page = SettingsPage::Rendering,
                    "settings-shortcuts" => self.settings_page = SettingsPage::Shortcuts,
                    "settings-styles" => self.settings_page = SettingsPage::Styles,
                    "settings-general" => self.settings_page = SettingsPage::General,
                    _ => {}
                }
            }
            "storyboard" | "studio" | "code" => {
                self.screen = Screen::Video;
                self.mode = match screen.as_str() {
                    "storyboard" => Mode::Storyboard,
                    "code" => Mode::Code,
                    _ => Mode::Studio,
                };
            }
            _ => {}
        }
        // `STUDIO_DEMO=menu|folder-menu|confirm|working|empty-folder` forces a transient state for screenshots.
        if let Ok(demo) = std::env::var("STUDIO_DEMO") {
            use crate::manage::RowTarget;
            match demo.as_str() {
                "menu" => self.row_menu = Some(RowTarget::Video(self.current.0, self.current.1)),
                "palette" => self.palette = true,
                "rail" => self.prefs.sidebar_collapsed = true,
                "folder-menu" => self.row_menu = Some(RowTarget::Project(self.current.0)),
                "confirm" => {
                    self.confirm = Some(crate::manage::Confirm {
                        target: RowTarget::Video(self.current.0, self.current.1),
                        title: format!("Delete “{}”?", self.video().title),
                        body: "The video, its scenes, narration and exports move to the Recycle Bin.".into(),
                        action: "Delete video",
                    })
                }
                "working" => {
                    let (pi, vi) = self.current;
                    self.projects[pi].videos[vi].chat.truncate(1);
                    self.projects[pi].videos[vi].chat.push(Msg::Working(model::Working {
                        label: "Writing scenes".into(),
                        since: Instant::now() - Duration::from_secs(42),
                        progress: Some((2, 6)),
                    }));
                }
                _ => {}
            }
        }
        if let Ok(step) = std::env::var("STUDIO_STEP") {
            self.onboarding_step = step.parse().unwrap_or(0).min(4);
        }
    }

    // ------------------------------------------------------------------
    // State helpers
    // ------------------------------------------------------------------

    pub fn video(&self) -> &model::Video {
        self.projects
            .get(self.current.0)
            .and_then(|p| p.videos.get(self.current.1))
            .unwrap_or_else(|| model::Video::placeholder())
    }

    /// Whether `current` points at a real video (false once every video is deleted).
    pub fn has_video(&self) -> bool {
        self.projects.get(self.current.0).is_some_and(|p| self.current.1 < p.videos.len())
    }

    /// The chat of the video an event belongs to.
    pub fn chat_of(&mut self, v: (usize, usize)) -> Option<&mut Vec<Msg>> {
        self.projects.get_mut(v.0).and_then(|p| p.videos.get_mut(v.1)).map(|v| &mut v.chat)
    }

    pub fn save_prefs(&self) {
        self.prefs.save();
    }

    pub fn go(&mut self, screen: Screen, cx: &mut Context<Self>) {
        // With no videos left, the composer stands in for the video screen.
        let screen = if screen == Screen::Video && !self.has_video() { Screen::Home } else { screen };
        self.screen = screen;
        self.open_menu = None;
        self.row_menu = None;
        cx.notify();
    }

    pub fn open_video(&mut self, project: usize, video: usize, cx: &mut Context<Self>) {
        if self.projects.get(project).is_none_or(|p| video >= p.videos.len()) {
            self.go(Screen::Home, cx);
            return;
        }
        let changed = self.current != (project, video);
        self.current = (project, video);
        if self.prefs.last_video != self.video().dir {
            self.prefs.last_video = self.video().dir.clone();
            self.save_prefs();
        }
        self.scene = 0;
        self.playhead = self.video().scenes.first().and_then(|s| s.engine.key_time).unwrap_or(0.);
        self.at_key_moment = true;
        self.playing = false;
        self.render_progress = self
            .jobs
            .iter()
            .find(|j| j.video == self.current && j.status.active())
            .map(|j| j.progress);
        self.mode = if self.video().status == VideoStatus::Draft { Mode::Storyboard } else { Mode::Studio };
        self.sync_preview();
        if changed {
            self.refresh_thumbs();
            self.refresh_voice(self.current, cx);
        }
        self.go(Screen::Video, cx);
    }

    pub fn toggle_menu(&mut self, key: &'static str, open: bool, cx: &mut Context<Self>) {
        self.look_hover = None;
        if open {
            let just_closed = matches!(self.menu_dismissed, Some((k, at)) if k == key && at.elapsed() < Duration::from_millis(400));
            self.menu_dismissed = None;
            if just_closed {
                return;
            }
            self.open_menu = Some(key);
        } else if self.open_menu == Some(key) {
            self.open_menu = None;
            self.menu_dismissed = Some((key, Instant::now()));
        }
        cx.notify();
    }

    pub fn menu_open(&self, key: &'static str) -> bool {
        self.open_menu == Some(key)
    }

    pub fn set_theme(&mut self, mode: theme::ThemeMode, window: &mut Window, cx: &mut Context<Self>) {
        self.prefs.theme = mode;
        self.save_prefs();
        theme::apply(mode, self.prefs.palette, window, cx);
        cx.notify();
    }

    /// The assistant new videos use; remembered across launches.
    pub fn set_default_provider(&mut self, provider: Provider) {
        self.default_provider = provider;
        self.prefs.default_provider = provider.key().to_string();
        self.save_prefs();
    }

    pub fn set_palette(&mut self, palette: theme::Palette, window: &mut Window, cx: &mut Context<Self>) {
        self.prefs.palette = palette;
        self.save_prefs();
        theme::apply(self.prefs.theme, palette, window, cx);
        cx.notify();
    }

    /// Select a scene: the stage shows its key moment (what its thumbnail shows).
    pub fn seek_scene(&mut self, i: usize, cx: &mut Context<Self>) {
        self.scene = i.min(self.video().scenes.len().saturating_sub(1));
        self.playing = false;
        if let Some(a) = &mut self.audio {
            a.stop();
        }
        self.show_key_moment();
        cx.notify();
    }

    /// Scrub the current scene to the fraction under window x-coordinate `x`.
    pub fn scrub_to(&mut self, x: gpui::Pixels, cx: &mut Context<Self>) {
        let b = self.scrub_bounds.get();
        if b.size.width <= gpui::px(0.) {
            return;
        }
        let f = ((x - b.origin.x) / b.size.width).clamp(0., 1.);
        let start = self.video().scene_start(self.scene);
        let Some(len) = self.video().scenes.get(self.scene).map(|s| s.length()) else { return };
        self.playhead = start + f * len;
        self.at_key_moment = false;
        if let Some(a) = &mut self.audio {
            a.stop();
        }
        self.sync_preview();
        cx.notify();
    }

    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if !self.playing && self.at_key_moment {
            self.playhead = self.video().scene_start(self.scene);
        }
        if self.playhead >= self.video().length() - 0.05 {
            self.playhead = 0.;
            self.scene = 0;
        }
        self.at_key_moment = false;
        self.playing = !self.playing;
        self.sync_preview();
        self.sync_audio(true);
        self.ensure_ticker(cx);
        cx.notify();
    }

    /// A frame-rate timer drives playback and stops itself when playback stops.
    /// (Render progress arrives as engine events, not ticks.)
    fn ensure_ticker(&mut self, cx: &mut Context<Self>) {
        if self.ticker.is_some() || !self.playing {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| {
            let mut last = Instant::now();
            loop {
                cx.background_executor().timer(Duration::from_millis(33)).await;
                let now = Instant::now();
                let dt = (now - last).as_secs_f32().min(0.25);
                last = now;
                let keep = this
                    .update(cx, |this, cx| {
                        this.tick(dt);
                        this.end_fade_if_done(cx);
                        cx.notify();
                        if !this.playing {
                            this.ticker = None;
                        }
                        this.playing
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    fn tick(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        if self.stage_loop && self.screen == Screen::Video {
            let start = self.video().scene_start(self.scene);
            let len = self.video().scenes.get(self.scene).map_or(0., |s| s.length());
            if len > 0. && self.playhead + dt >= start + len {
                self.playhead = start;
                self.sync_audio(true);
                self.sync_preview();
                return;
            }
        }
        let total = self.video().length();
        self.playhead = (self.playhead + dt).min(total);
        let mut acc = 0.;
        for (i, s) in self.video().scenes.iter().enumerate() {
            acc += s.length();
            if self.playhead < acc {
                self.scene = i;
                break;
            }
        }
        self.sync_audio(false);
        if self.playhead >= total {
            if self.screen == Screen::Onboarding {
                // The onboarding demo loops.
                self.playhead = 0.;
                self.scene = 0;
            } else {
                self.playing = false;
            }
        }
        self.sync_preview();
    }

    /// Onboarding's right half plays the sample lesson, rendered live by the engine.
    pub fn start_onboarding_demo(&mut self, cx: &mut Context<Self>) {
        let Some(video) = self.sample_video() else { return };
        self.current = video;
        self.scene = 0;
        self.playhead = 0.;
        self.at_key_moment = false;
        self.playing = true;
        self.sync_preview();
        self.ensure_ticker(cx);
    }

    /// The stage gain: 0 when muted.
    pub fn stage_volume(&self) -> f32 {
        if self.prefs.muted { 0. } else { self.prefs.volume.clamp(0., 1.) }
    }

    pub fn apply_volume(&mut self) {
        let v = self.stage_volume();
        if let Some(a) = &mut self.audio {
            a.set_volume(v);
        }
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        // Unmuting at zero volume would stay silent, so bring it back to a usable level.
        if self.prefs.muted || self.prefs.volume <= 0.01 {
            self.prefs.muted = false;
            if self.prefs.volume <= 0.01 {
                self.prefs.volume = 0.6;
            }
        } else {
            self.prefs.muted = true;
        }
        self.apply_volume();
        self.save_prefs();
        cx.notify();
    }

    /// Use the length typed in the options panel, if it reads as one.
    pub fn commit_length(&mut self, cx: &mut Context<Self>) {
        let text = self.length_input.read(cx).text().to_string();
        if text.trim().is_empty() {
            return;
        }
        match crate::model::parse_length(&text) {
            Some(secs) => {
                self.prefs.video_seconds = secs;
                self.save_prefs();
                self.length_input.update(cx, |i, cx| i.clear(cx));
            }
            None => self.notice = Some(format!("“{}” isn't a length. Try 3:30, 7 min or 90s.", text.trim())),
        }
        cx.notify();
    }

    /// Raise or lower the volume by `d` (keyboard).
    pub fn nudge_volume(&mut self, d: f32, cx: &mut Context<Self>) {
        let from = self.stage_volume();
        self.prefs.volume = ((from + d) * 10.).round() / 10.;
        self.prefs.volume = self.prefs.volume.clamp(0., 1.);
        self.prefs.muted = false;
        self.apply_volume();
        self.save_prefs();
        cx.notify();
    }

    /// Show Storyboard, Studio or Code for the open video.
    pub fn switch_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.screen != Screen::Video && self.screen != Screen::Onboarding {
            self.go(Screen::Video, cx);
        }
        if self.screen != Screen::Video || self.mode == mode {
            return;
        }
        self.mode = mode;
        if mode == Mode::Studio {
            self.sync_preview();
        }
        cx.notify();
    }

    /// Set the volume from window x-coordinate `x` over the volume slider.
    pub fn volume_to(&mut self, x: gpui::Pixels, cx: &mut Context<Self>) {
        let b = self.volume_bounds.get();
        if b.size.width <= gpui::px(0.) {
            return;
        }
        let f = ((x - b.origin.x) / b.size.width).clamp(0., 1.);
        // The ends are small targets: snap the last few pixels to silent / full.
        self.prefs.volume = if f > 0.94 { 1. } else if f < 0.04 { 0. } else { f };
        self.prefs.muted = false;
        self.apply_volume();
        cx.notify();
    }

    /// Keep narration playback in step with the stage: play the current scene's line
    /// from the playhead while playing (restart when `force`), stop otherwise.
    pub fn sync_audio(&mut self, force: bool) {
        let key = (self.current, self.scene);
        let voice = self.video().scenes.get(self.scene).and_then(|s| s.engine.voice.clone());
        let local = self.playhead - self.video().scene_start(self.scene);
        let muted = self.screen == Screen::Onboarding;
        let Some(audio) = &mut self.audio else { return };
        match (self.playing && !muted, voice) {
            (true, Some(v)) if local < v.seconds => {
                let same = audio.current.as_ref().is_some_and(|(vk, s, w)| (*vk, *s) == key && *w == v.wav);
                if force || !same {
                    audio.play(key, &v.wav, local);
                }
            }
            (true, _) => {
                if audio.current.as_ref().is_some_and(|(vk, s, _)| (*vk, *s) != key) {
                    audio.stop();
                }
            }
            (false, _) => audio.stop(),
        }
    }

    /// Start editing scene `i`'s narration line.
    pub fn edit_narration_line(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.video().scenes.get(i).map(|s| s.say.clone()) else { return };
        self.editing_line = Some(i);
        self.narration_input.update(cx, |input, cx| input.set_text(text, cx));
        let handle = self.narration_input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Save the edited line into the storyboard, then re-speak and re-time.
    pub fn save_narration_line(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(i) = self.editing_line.take() else { return };
        let text = text.trim().to_string();
        let video = self.current;
        let v = &mut self.projects[video.0].videos[video.1];
        if let Some(s) = v.scenes.get_mut(i) {
            s.say = text.clone();
        }
        if let Some(beat) = v.storyboard.as_mut().and_then(|sb| sb.scenes.get_mut(i)) {
            beat.say = text;
        }
        self.persist(video);
        self.refresh_voice(video, cx);
        cx.notify();
    }

    pub fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        self.playing = false;
        self.prefs.onboarded = true;
        self.save_prefs();
        self.go(Screen::Home, cx);
    }

    pub fn sidebar_visible(&self, window: &Window) -> bool {
        !self.prefs.sidebar_collapsed && !theme::is_compact(window)
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

// ----------------------------------------------------------------------
// Small shared builders
// ----------------------------------------------------------------------

/// Adapt a `cx.listener` to callbacks that pass a `Copy` value.
pub fn with_value<V: Copy + 'static>(
    cx: &Context<Workspace>,
    f: impl Fn(&mut Workspace, V, &mut Window, &mut Context<Workspace>) + 'static,
) -> impl Fn(V, &mut Window, &mut App) + 'static {
    let l = cx.listener(move |this, v: &V, w, cx| f(this, *v, w, cx));
    move |v, w, cx| l(&v, w, cx)
}

pub fn click(
    cx: &Context<Workspace>,
    f: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, _: &ClickEvent, w, cx| f(this, w, cx))
}

pub fn display(s: impl Into<SharedString>) -> Div {
    div()
        .font_family(theme::FONT_DISPLAY)
        .text_size(text::DISPLAY)
        .line_height(px(34.))
        .font_weight(FontWeight::SEMIBOLD)
        .letter_spacing(px(-0.2))
        .child(s.into())
}

pub fn title(s: impl Into<SharedString>) -> Div {
    div()
        .text_size(text::TITLE)
        .line_height(px(22.))
        .font_weight(FontWeight::SEMIBOLD)
        .letter_spacing(px(0.))
        .child(s.into())
}

pub fn strong(s: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::MEDIUM).child(s.into())
}

pub fn faint(s: impl Into<SharedString>, cx: &App) -> Div {
    div().text_size(text::SMALL).text_color(cx.theme().fg_3).child(s.into())
}

pub fn sub(s: impl Into<SharedString>, cx: &App) -> Div {
    div().text_color(cx.theme().fg_2).child(s.into())
}

pub fn num(s: impl Into<SharedString>) -> Div {
    div().font_features(num_features()).child(s.into())
}

/// Empty titlebar area that drags the window and maximizes on double-click.
pub fn drag_area(id: &'static str) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .h_full()
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |ev, window, _| {
            if ev.click_count == 2 {
                window.titlebar_double_click();
            } else if cfg!(target_os = "linux") {
                window.start_window_move();
            }
        })
}

/// Clicking anywhere in a composer box puts the caret in its input.
pub fn focus_on_click(input: &Entity<TextInput>) -> impl Fn(&gpui::MouseDownEvent, &mut Window, &mut App) + 'static {
    let input = input.clone();
    move |_, window, cx| {
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }
}

pub fn stage_image(art: studio_ui::assets::Art) -> impl IntoElement {
    gpui::img(art.path())
        .size_full()
        .object_fit(gpui::ObjectFit::Cover)
}

/// An engine frame or thumbnail.
pub fn still_image(image: std::sync::Arc<gpui::RenderImage>) -> impl IntoElement {
    gpui::img(gpui::ImageSource::Render(image))
        .size_full()
        .object_fit(gpui::ObjectFit::Cover)
}

/// The sample lesson in house style `i` when the engine has drawn it, else placeholder art.
pub fn style_image(ws: &Workspace, i: usize, art: studio_ui::assets::Art) -> AnyElement {
    if let Some(model::Still(img)) = ws.style_clip_frame(i) {
        return still_image(img).into_any_element();
    }
    match ws.style_previews.get(i).cloned().flatten() {
        Some(model::Still(img)) => still_image(img).into_any_element(),
        None => stage_image(art).into_any_element(),
    }
}

/// A scene's engine thumbnail when it has one. A scripted scene without one is still
/// running or failed (blank, or an error mark); only unbuilt scenes show placeholder art.
pub fn scene_image(scene: &model::Scene, cx: &App) -> AnyElement {
    let t = cx.theme();
    match (&scene.engine.thumb, &scene.engine.file) {
        (Some(model::Still(img)), _) => still_image(img.clone()).into_any_element(),
        (None, Some(_)) => div()
            .size_full()
            .bg(t.stage)
            .flex()
            .items_center()
            .justify_center()
            .when(matches!(scene.engine.issue, Some(model::Issue::Error { .. })), |d| {
                d.child(Icon::new(IconName::Info).small().color(t.err))
            })
            .into_any_element(),
        // Not built yet: a neutral sketch tile (no stand-in art that could mislead).
        (None, None) => div()
            .size_full()
            .bg(t.surface_3)
            .flex()
            .items_center()
            .justify_center()
            .child(Icon::new(IconName::Board).color(t.fg_3))
            .into_any_element(),
    }
}


// ----------------------------------------------------------------------
// Root render
// ----------------------------------------------------------------------

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let content: AnyElement = match self.screen {
            Screen::Onboarding => self.render_onboarding(window, cx),
            Screen::Home => self.render_home(window, cx),
            Screen::Video => match self.mode {
                Mode::Storyboard => self.render_storyboard(window, cx),
                Mode::Studio => self.render_studio(window, cx),
                Mode::Code => self.render_code(window, cx),
            },
            Screen::Renders => self.render_renders(window, cx),
            Screen::Settings => self.render_settings(window, cx),
        };
        let sidebar_allowed = self.screen != Screen::Onboarding && self.screen != Screen::Settings;
        let show_sidebar = sidebar_allowed && self.sidebar_visible(window);

        div()
            .id("workspace")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.window)
            .text_color(t.fg)
            .font_family(FONT_UI)
            .font_features(ui_features())
            .text_size(text::BODY)
            .line_height(text::LEADING)
            .letter_spacing(text::TRACKING)
            .on_action(cx.listener(|this, _: &RenderVideo, _, cx| {
                if this.screen == Screen::Video {
                    this.start_render(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NewVideo, _, cx| this.go(Screen::Home, cx)))
            .on_action(|_: &crate::shortcuts::FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &crate::shortcuts::FocusPrev, window, cx| window.focus_prev(cx))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| this.go(Screen::Settings, cx)))
            .on_action(cx.listener(|this, _: &OpenRenders, _, cx| this.go(Screen::Renders, cx)))
            .key_context(crate::shortcuts::CONTEXT)
            // Leaving the new-video options panel hides its look preview.
            .on_mouse_move(cx.listener(|this, _, _, cx| this.clear_look_hover(cx)))
            .on_action(cx.listener(|this, _: &TogglePlay, _, cx| {
                if this.screen == Screen::Video {
                    this.toggle_play(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleMute, _, cx| this.toggle_mute(cx)))
            .on_action(cx.listener(|this, _: &VolumeUp, _, cx| this.nudge_volume(0.1, cx)))
            .on_action(cx.listener(|this, _: &VolumeDown, _, cx| this.nudge_volume(-0.1, cx)))
            .on_action(cx.listener(|this, _: &PrevScene, _, cx| {
                if this.screen == Screen::Video && this.scene > 0 {
                    this.seek_scene(this.scene - 1, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NextScene, _, cx| {
                if this.screen == Screen::Video {
                    this.seek_scene(this.scene + 1, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &ModeStoryboard, _, cx| this.switch_mode(Mode::Storyboard, cx)))
            .on_action(cx.listener(|this, _: &ModeStudio, _, cx| this.switch_mode(Mode::Studio, cx)))
            .on_action(cx.listener(|this, _: &ModeCode, _, cx| this.switch_mode(Mode::Code, cx)))
            .on_action(cx.listener(|this, _: &ToggleCaptions, _, cx| {
                this.stage_captions = !this.stage_captions;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSafeArea, _, cx| {
                this.stage_safe = !this.stage_safe;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleLoop, _, cx| {
                this.stage_loop = !this.stage_loop;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                if this.palette {
                    this.close_palette(cx);
                } else {
                    this.open_palette(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &crate::shortcuts::PaletteUp, _, cx| this.palette_move(-1, cx)))
            .on_action(cx.listener(|this, _: &crate::shortcuts::PaletteDown, _, cx| this.palette_move(1, cx)))
            .on_action(cx.listener(|this, _: &CloseMenus, _, cx| {
                this.open_menu = None;
                this.row_menu = None;
                this.confirm = None;
                this.palette = false;
                this.cancel_rename(cx);
                this.editing_line = None;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.prefs.sidebar_collapsed = !this.prefs.sidebar_collapsed;
                this.save_prefs();
                cx.notify();
            }))
            .child(self.render_titlebar(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .px(space::GUTTER)
                    .pb(space::GUTTER)
                    .when(sidebar_allowed, |d| {
                        // Collapsing slides the sidebar shut instead of snapping.
                        let sidebar = self.render_sidebar(window, cx);
                        d.child(
                            div()
                                .flex_none()
                                .h_full()
                                .overflow_hidden()
                                .with_spring("sidebar-slide", motion::spring_to(if show_sidebar { 1. } else { 0. }), move |d, v| {
                                    // Slide the whole panel out to the left behind a clip. No
                                    // opacity: fading a layered panel flashes its fills.
                                    let v = v.clamp(0., 1.);
                                    d.w(space::SIDEBAR * v).mr(space::GUTTER * v).child(
                                        div()
                                            .relative()
                                            .left(-space::SIDEBAR * (1. - v))
                                            .w(space::SIDEBAR)
                                            .h_full()
                                            .flex()
                                            .child(sidebar),
                                    )
                                }),
                        )
                        .when(!show_sidebar, |d| {
                            let rail = self.render_rail(cx);
                            d.child(div().flex_none().mr(space::GUTTER).h_full().child(rail))
                        })
                    })
                    .child(motion::enter(
                        div().flex_1().min_w_0().min_h_0().flex().child(content),
                        (SharedString::from(format!("screen-{:?}-{:?}", self.screen, self.mode)), 0usize),
                        motion::BASE,
                        6.,
                    )),
            )
            .when_some(self.notice.clone(), |d, notice| {
                let t = cx.theme().clone();
                d.child(
                    div().absolute().top(space::TITLEBAR + space::GUTTER).left_0().right_0().flex().justify_center().child(
                        floating(cx)
                            .max_w(px(520.))
                            .gap(px(10.))
                            .px(px(12.))
                            .py(px(8.))
                            .rounded(radius::PANEL)
                            .child(Icon::new(IconName::Info).small().color(t.fg_2))
                            .child(div().flex_1().min_w_0().child(notice))
                            .child(Button::icon("notice-close", IconName::X).small().on_click(click(cx, |this, _, cx| {
                                this.notice = None;
                                cx.notify();
                            }))),
                    ),
                )
            })
            .when_some(self.confirm.clone(), |d, c| d.child(self.render_confirm(c, cx)))
            .when(self.palette, |d| d.child(self.render_palette(cx)))
            .when(self.toast && self.screen == Screen::Video, |d| {
                let top = space::TITLEBAR + space::GUTTER * 2.;
                d.child(self.render_toast(cx).with_animation("toast", motion::ease(motion::SLOW), move |el, v| {
                    el.opacity(v).top(top - px((1. - v) * 10.))
                }))
            })
    }
}

impl Workspace {
    /// The delete confirmation: a dimmed backdrop (click it to cancel) and a card.
    fn render_confirm(&self, c: crate::manage::Confirm, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let card = div()
            .id("confirm-card")
            .occlude()
            .w(px(380.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(18.))
            .rounded(radius::PANEL)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow(t.pop_shadow())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(title(c.title))
            .child(sub(c.body, cx))
            .child(
                div()
                    .mt(px(14.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(Button::new("confirm-cancel", "Cancel").on_click(click(cx, |this, _, cx| {
                        this.confirm = None;
                        cx.notify();
                    })))
                    .child(
                        Button::new("confirm-delete", c.action)
                            .danger()
                            .with_icon(IconName::Trash)
                            .on_click(click(cx, |this, _, cx| this.confirm_delete(cx))),
                    ),
            );
        div()
            .id("confirm-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.confirm = None;
                cx.notify();
            }))
            .child(motion::fade(div().absolute().top_0().left_0().size_full().bg(gpui::hsla(0., 0., 0., 0.45)), "confirm-dim", motion::BASE))
            .child(motion::rise(card, "confirm-rise"))
    }

    fn render_toast(&mut self, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        // Sits over the stage's top-right corner so it never covers the composer.
        floating(cx)
            .absolute()
            .right(space::RIGHT_PANEL + space::GUTTER * 3.)
            .w(px(330.))
            .items_start()
            .gap(px(12.))
            .px(px(14.))
            .py(px(12.))
            .rounded(radius::PANEL)
            .child(div().pt(px(2.)).child(Icon::new(IconName::Download).color(t.fg_2)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(strong(match &self.update {
                        crate::update::UpdateState::Ready { version, .. } => format!("Studio {version} is ready"),
                        _ => "An update is ready".to_string(),
                    }))
                    .child(faint(
                        "Downloaded and verified in the background. It applies next time you open Studio, or restart now.",
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .mt(px(10.))
                            .child(Button::new("toast-restart", "Restart now").primary().small().on_click(click(cx, |this, _, cx| {
                                this.toast = false;
                                this.restart_to_update(cx);
                            })))
                            .child(Button::new("toast-notes", "What's new").ghost().small().on_click(click(cx, |this, _, cx| {
                                if let crate::update::UpdateState::Ready { notes, .. } = &this.update
                                    && !notes.is_empty() {
                                        this.notice = Some(notes.clone());
                                    }
                                this.toast = false;
                                cx.notify();
                            }))),
                    ),
            )
            .child(Button::icon("toast-close", IconName::X).small().on_click(click(cx, |this, _, cx| {
                this.toast = false;
                cx.notify();
            })))
    }

    pub fn render_chip_for(status: &JobStatus) -> Chip {
        match status {
            JobStatus::Rendering { scene, of } => {
                Chip::new(format!("Rendering scene {scene} / {of}")).tone(Tone::Signal)
            }
            JobStatus::Exporting => Chip::new("Joining scenes").tone(Tone::Signal),
            JobStatus::Waiting(n) => Chip::new(format!("Waiting · #{n}")),
            JobStatus::Done(d) => Chip::new(format!("Done · {d}")).tone(Tone::Ok).icon(IconName::Check),
            JobStatus::Failed(r) => Chip::new(format!("Failed · {r}")).tone(Tone::Err),
            JobStatus::Cancelled => Chip::new("Cancelled"),
        }
    }

    pub fn status_dot(status: VideoStatus) -> Dot {
        Dot::new(match status {
            VideoStatus::Draft => Tone::Neutral,
            VideoStatus::Building => Tone::Signal,
            VideoStatus::Ready => Tone::Ok,
            VideoStatus::Failed => Tone::Err,
        })
    }

    pub fn mode_switch(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = match self.mode {
            Mode::Storyboard => 0,
            Mode::Studio => 1,
            Mode::Code => 2,
        };
        Segmented::new(
            "mode",
            vec![
                Segment::new("Storyboard").icon(IconName::Board),
                Segment::new("Studio").icon(IconName::Film),
                Segment::new("Code").icon(IconName::Script),
            ],
            selected,
        )
        .hide_icons(compact)
        .on_select(with_value(cx, |this, i: usize, _, cx| {
            this.mode = [Mode::Storyboard, Mode::Studio, Mode::Code][i];
            cx.notify();
        }))
    }
}
