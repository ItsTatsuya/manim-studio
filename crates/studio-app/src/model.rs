//! Projects, videos and scenes as the workspace holds them. They load from and save to
//! the video folders on disk (see `store`).

use gpui::RenderImage;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use studio_ui::IconName;
use studio_ui::assets::Art;
use studio_agent::prompt::{Beat, Storyboard};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Claude,
    OpenAi,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::OpenAi => "ChatGPT",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Provider::Claude => IconName::Claude,
            Provider::OpenAi => IconName::OpenAi,
        }
    }

    pub fn cli(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::OpenAi => "Codex",
        }
    }

    pub fn agent(self) -> studio_agent::Provider {
        match self {
            Provider::Claude => studio_agent::Provider::Claude,
            Provider::OpenAi => studio_agent::Provider::Codex,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::OpenAi => "codex",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Provider::Claude),
            "codex" => Some(Provider::OpenAi),
            _ => None,
        }
    }
}

/// Model size, independent of provider. Settings store the tier, so switching
/// assistant keeps the same cost/quality trade-off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    Fast,
    Balanced,
    Best,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Fast, Tier::Balanced, Tier::Best];

    pub fn ix(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(1)
    }

    pub fn model(self, p: Provider) -> &'static str {
        match (p, self) {
            (Provider::Claude, Tier::Fast) => "Haiku",
            (Provider::Claude, Tier::Balanced) => "Sonnet",
            (Provider::Claude, Tier::Best) => "Opus",
            // Codex uses the model the teacher's ChatGPT plan gives it; the tier sets how
            // hard it thinks instead.
            (Provider::OpenAi, Tier::Fast) => "Codex · light",
            (Provider::OpenAi, Tier::Balanced) => "Codex",
            (Provider::OpenAi, Tier::Best) => "Codex · deep",
        }
    }

    pub fn size(self) -> studio_agent::Size {
        match self {
            Tier::Fast => studio_agent::Size::Small,
            Tier::Balanced => studio_agent::Size::Medium,
            Tier::Best => studio_agent::Size::Large,
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Tier::Fast => "Quick edits and questions",
            Tier::Balanced => "Best for building scenes",
            Tier::Best => "Hardest topics, uses more plan",
        }
    }
}

/// How long the model thinks before answering. Maps to Claude's effort level
/// and Codex's reasoning effort.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reasoning {
    Auto,
    Low,
    Medium,
    High,
    Max,
}

impl Reasoning {
    pub const ALL: [Reasoning; 5] = [Reasoning::Auto, Reasoning::Low, Reasoning::Medium, Reasoning::High, Reasoning::Max];

    pub fn ix(self) -> usize {
        Self::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }

    /// The effort to ask for. `Auto` lets the CLI decide, except where Studio knows
    /// better (`auto_default`: low for plans, high for repairs).
    pub fn effort(self, auto_default: Option<studio_agent::Effort>) -> Option<studio_agent::Effort> {
        use studio_agent::Effort;
        match self {
            Reasoning::Auto => auto_default,
            Reasoning::Low => Some(Effort::Low),
            Reasoning::Medium => Some(Effort::Medium),
            Reasoning::High => Some(Effort::High),
            Reasoning::Max => Some(Effort::Max),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Reasoning::Auto => "Auto",
            Reasoning::Low => "Low",
            Reasoning::Medium => "Medium",
            Reasoning::High => "High",
            Reasoning::Max => "Max",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Reasoning::Auto => "Light for questions, deeper for new scenes and repairs",
            Reasoning::Low => "Fastest and cheapest, fine for small tweaks",
            Reasoning::Medium => "Good default for most scenes",
            Reasoning::High => "Tricky animations and maths",
            Reasoning::Max => "Slow and expensive, for last resorts",
        }
    }
}

/// Ask before a single turn spends more than this many tokens.
pub const TURN_BUDGETS: [(&str, Option<u32>); 4] =
    [("10k tokens", Some(10_000)), ("25k tokens", Some(25_000)), ("50k tokens", Some(50_000)), ("No limit", None)];

/// What the app knows about an assistant CLI on this computer.
#[derive(Clone, Debug)]
pub struct ProviderStatus {
    pub provider: Provider,
    pub installed: bool,
    pub version: Option<String>,
    pub signed_in: bool,
    pub plan: Option<String>,
    /// Plan usage (0..=1) from the last turn's rate-limit report.
    pub window_used: Option<f32>,
    pub week_used: Option<f32>,
    pub resets_at: Option<u64>,
    pub week_resets_at: Option<u64>,
    /// Detection is running.
    pub checking: bool,
}

impl ProviderStatus {
    pub fn unknown(provider: Provider) -> Self {
        ProviderStatus {
            provider,
            installed: false,
            version: None,
            signed_in: false,
            plan: None,
            window_used: None,
            week_used: None,
            resets_at: None,
            week_resets_at: None,
            checking: true,
        }
    }

    /// "Max", "Pro", ... or the provider name when the plan is unknown.
    pub fn plan_label(&self) -> String {
        self.plan.clone().unwrap_or_else(|| "plan".into())
    }

    /// "3 h 12 min" until the 5-hour window resets.
    pub fn resets_in(&self) -> Option<String> {
        until(self.resets_at?)
    }

    /// "2 d 4 h" until the weekly window resets.
    pub fn week_resets_in(&self) -> Option<String> {
        until(self.week_resets_at?)
    }
}

/// Time left until `at` (Unix seconds): "2 d 4 h", "3 h 12 min", "8 min". `None` once it
/// has passed (the plan has reset, the stored figure is stale).
pub fn until(at: u64) -> Option<String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let left = at.checked_sub(now).filter(|l| *l > 0)?;
    Some(if left >= 86_400 {
        format!("{} d {} h", left / 86_400, left % 86_400 / 3600)
    } else if left >= 3600 {
        format!("{} h {} min", left / 3600, left % 3600 / 60)
    } else {
        format!("{} min", (left / 60).max(1))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoStatus {
    Draft,
    Building,
    Ready,
    Failed,
}

impl VideoStatus {
    pub fn key(self) -> &'static str {
        match self {
            VideoStatus::Draft => "draft",
            VideoStatus::Building => "building",
            VideoStatus::Ready => "ready",
            VideoStatus::Failed => "failed",
        }
    }

    pub fn from_key(s: &str) -> Self {
        match s {
            "ready" => VideoStatus::Ready,
            "failed" => VideoStatus::Failed,
            // A build interrupted by quitting is a draft again.
            _ => VideoStatus::Draft,
        }
    }
}

/// An engine-rendered still (scene thumbnail or stage frame).
#[derive(Clone)]
pub struct Still(pub Arc<RenderImage>);

impl std::fmt::Debug for Still {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Still")
    }
}

/// What the engine last said about a scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Issue {
    /// The script failed to load or run: first line, and the TypeScript line if known.
    Error { message: String, line: Option<u32> },
    /// It runs, but the layout check found problems (overlap, off-frame text, ghosting).
    Layout { count: usize, first: String },
}

/// Engine-side state of a scene: its script and what running it produced.
#[derive(Clone, Debug, Default)]
pub struct SceneEngine {
    pub file: Option<PathBuf>,
    /// Real duration from the last run, in seconds.
    pub duration: Option<f32>,
    /// The scene's key moment (its thumbnail): a `mark("thumbnail")` or the end of the
    /// last animation. Selecting the scene shows this.
    pub key_time: Option<f32>,
    pub thumb: Option<Still>,
    pub issue: Option<Issue>,
    /// Whether the scene has run at least once (check result is known).
    pub checked: bool,
    /// The narration line, spoken and measured.
    pub voice: Option<VoiceLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoiceLine {
    /// The scene's whole narration track: its sentences laid out where the scene places
    /// them (`this.narrate(n)`), else one after another.
    pub wav: PathBuf,
    pub seconds: f32,
    /// The text it speaks (to tell when it is stale).
    pub text: String,
    /// Each sentence, spoken and cached on its own.
    pub parts: Vec<VoicePart>,
    /// Where each sentence starts in the track (seconds).
    pub starts: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoicePart {
    pub wav: PathBuf,
    pub seconds: f32,
    pub text: String,
}

impl VoiceLine {
    /// Sentence lengths, as the scene sees them in `NARRATION.lines`.
    pub fn lines(&self) -> Vec<f64> {
        self.parts.iter().map(|p| p.seconds as f64).collect()
    }
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub title: String,
    /// Narration.
    pub say: String,
    /// What appears on screen.
    pub visual: String,
    /// Planned length from the storyboard.
    pub seconds: u32,
    pub art: Art,
    pub note: Option<String>,
    pub engine: SceneEngine,
}

impl Scene {
    /// Real length when the engine has run the scene, else the planned one.
    pub fn length(&self) -> f32 {
        self.engine.duration.unwrap_or(self.seconds as f32)
    }

    pub fn from_beat(i: usize, b: &Beat) -> Self {
        Scene {
            title: b.title.clone(),
            say: b.say.clone(),
            visual: b.visual.clone(),
            seconds: b.seconds,
            art: Art::ALL[i % Art::ALL.len()],
            note: None,
            engine: SceneEngine::default(),
        }
    }
}

/// Per-video overrides, saved in `video.json`. `None` follows the defaults in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoSettings {
    /// Index into `studio_render::style::STYLES`.
    pub style: Option<usize>,
    /// Index into `engine::OUTPUTS`.
    pub output: Option<usize>,
}

/// The assistant is working on this video.
#[derive(Clone, Debug, PartialEq)]
pub enum Busy {
    Planning,
    Revising,
    Building,
    Editing,
    /// Repairing scene `n` (1-based), try `k` of `of`.
    Fixing { scene: usize, k: u8, of: u8 },
}

#[derive(Clone, Debug)]
pub struct Video {
    pub title: String,
    pub status: VideoStatus,
    /// Last change (Unix seconds).
    pub updated: u64,
    pub scenes: Vec<Scene>,
    pub source: Option<String>,
    pub grade: String,
    /// The video folder (video.json, scenes/, lib/, assets/, .cache/, exports/).
    pub dir: Option<PathBuf>,
    pub settings: VideoSettings,
    /// The approved or draft plan.
    pub storyboard: Option<Storyboard>,
    /// The assistant conversation to resume, and which CLI it belongs to.
    pub session: Option<(Provider, String)>,
    pub chat: Vec<Msg>,
    pub busy: Option<Busy>,
    /// The teacher's original request (shown while planning).
    pub request: Option<String>,
    /// The next chat turn continues the last conversation (after a budget pause).
    pub resume_next: bool,
}

impl Video {
    /// Stands in for the open video when every video has been deleted, so read-only
    /// views have something to look at. Never saved or edited.
    pub fn placeholder() -> &'static Video {
        static EMPTY: std::sync::OnceLock<Video> = std::sync::OnceLock::new();
        EMPTY.get_or_init(|| Video {
            title: String::new(),
            status: VideoStatus::Draft,
            updated: 0,
            scenes: Vec::new(),
            source: None,
            grade: String::new(),
            dir: None,
            settings: VideoSettings::default(),
            storyboard: None,
            session: None,
            chat: Vec::new(),
            busy: None,
            request: None,
            resume_next: false,
        })
    }

    pub fn seconds(&self) -> u32 {
        self.length().round() as u32
    }

    pub fn length(&self) -> f32 {
        self.scenes.iter().map(|s| s.length()).sum()
    }

    /// Start time of scene `i` within the video.
    pub fn scene_start(&self, i: usize) -> f32 {
        self.scenes[..i.min(self.scenes.len())].iter().map(|s| s.length()).sum()
    }

    /// Whether the engine can preview and render this video.
    pub fn has_scripts(&self) -> bool {
        self.dir.is_some() && self.scenes.iter().any(|s| s.engine.file.is_some())
    }

    pub fn cover_still(&self) -> Option<Still> {
        self.scenes.iter().find_map(|s| s.engine.thumb.clone())
    }

    pub fn cover(&self) -> Art {
        self.scenes.first().map_or(Art::Intro, |s| s.art)
    }

    /// "now", "5 min", "2 h", "3d".
    pub fn updated_label(&self) -> String {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let ago = now.saturating_sub(self.updated);
        match ago {
            0..120 => "now".into(),
            120..3600 => format!("{} min", ago / 60),
            3600..86400 => format!("{} h", ago / 3600),
            _ => format!("{}d", ago / 86400),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub expanded: bool,
    pub videos: Vec<Video>,
    pub dir: PathBuf,
    /// Export this project's videos without the default watermark.
    pub no_watermark: bool,
}

#[derive(Clone, Debug)]
pub enum Msg {
    User(String),
    Assistant(String),
    Activity {
        icon: IconName,
        ok: bool,
        text: String,
        meta: String,
    },
    /// The assistant is working: one live status line.
    Working(Working),
    Rendering,
    Usage(String),
}

/// What the live status line shows while the assistant works.
#[derive(Clone, Debug)]
pub struct Working {
    /// "Writing scenes", "Checking 6 scenes", "Fixing scene 3 · try 1 of 2".
    pub label: String,
    /// When the work started (kept across label changes, so the timer keeps counting).
    pub since: std::time::Instant,
    /// Known progress (done, of), e.g. scene files written during a build.
    pub progress: Option<(usize, usize)>,
}

impl Msg {
    /// Rows that narrate the assistant's internals (files it touched, checks that
    /// passed). Older chats saved them; they are no longer shown.
    pub fn is_work_detail(&self) -> bool {
        match self {
            Msg::Activity { ok, text, .. } => {
                text.starts_with("Edited ") || text.starts_with("Wrote ") || (*ok && text.contains("check passed"))
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobStatus {
    /// Position in the queue (1 = next).
    Waiting(usize),
    Rendering { scene: usize, of: usize },
    Exporting,
    /// Finished: the video's length.
    Done(String),
    Failed(String),
    Cancelled,
}

impl JobStatus {
    pub fn active(&self) -> bool {
        matches!(self, JobStatus::Waiting(_) | JobStatus::Rendering { .. } | JobStatus::Exporting)
    }
}

#[derive(Clone, Debug)]
pub struct RenderJob {
    pub id: u64,
    pub title: String,
    pub project: String,
    pub output: String,
    pub status: JobStatus,
    pub art: Art,
    pub thumb: Option<Still>,
    /// Wall-clock render time once finished.
    pub time: String,
    pub progress: f32,
    /// Which video, as (project, video) indices.
    pub video: (usize, usize),
    pub path: Option<PathBuf>,
    /// Scenes reused from the cache / rendered fresh.
    pub cached: usize,
    pub rendered: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct HouseStyle {
    pub name: &'static str,
    pub blurb: &'static str,
    pub art: Art,
}

/// Same order as `studio_render::style::STYLES`.
pub const HOUSE_STYLES: [HouseStyle; 6] = [
    HouseStyle { name: "Slate", blurb: "Chalkboard with serif titles, for maths and physics", art: Art::Circle },
    HouseStyle { name: "Whiteboard", blurb: "Marker colours and hand-lettered titles, for tutoring", art: Art::Photo },
    HouseStyle { name: "Clean", blurb: "Bright and modern, for explainers and training", art: Art::Frac },
    HouseStyle { name: "Phosphor", blurb: "Amber terminal, for programming and computing", art: Art::Vec },
    HouseStyle { name: "Playground", blurb: "Round, warm and big, for ages 6 to 11", art: Art::Quiz },
    HouseStyle { name: "Documentary", blurb: "Cinematic serif, for history and storytelling", art: Art::Graph },
];

/// Lesson length presets for new videos (Home → options).
/// One choice in the new-video options: what the chip shows, what the menu explains,
/// and what the planning prompt is told.
pub struct PlanChoice {
    pub name: &'static str,
    /// Chip label when picked.
    pub short: &'static str,
    pub blurb: &'static str,
    pub guide: &'static str,
}

const fn choice(name: &'static str, short: &'static str, blurb: &'static str, guide: &'static str) -> PlanChoice {
    PlanChoice { name, short, blurb, guide }
}

/// Preset target lengths for new videos, in seconds. Any other length can be typed.
pub const LENGTHS: [u32; 6] = [30, 60, 120, 240, 360, 600];
/// Typed lengths are kept within this range (seconds).
pub const LENGTH_RANGE: (u32, u32) = (15, 20 * 60);

/// "45 s", "2 min", "3:30".
pub fn length_label(secs: u32) -> String {
    if secs < 60 {
        format!("{secs} s")
    } else if secs.is_multiple_of(60) {
        format!("{} min", secs / 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// Read a typed length: "3:30", "7", "7 min", "90", "90s", "1.5 m", "2m 30s". A bare
/// number up to 20 is minutes, above that seconds. `None` when it isn't a length.
pub fn parse_length(text: &str) -> Option<u32> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    let secs = if let Some((m, s)) = t.split_once(':') {
        m.trim().parse::<u32>().ok()? * 60 + s.trim().parse::<u32>().ok()?
    } else {
        // Sum "<number><unit>" parts; a number without a unit is minutes.
        let mut total = 0.0f32;
        let mut num = String::new();
        let mut rest = t.as_str();
        let mut any = false;
        while !rest.is_empty() {
            let c = rest.chars().next()?;
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
                rest = &rest[c.len_utf8()..];
                continue;
            }
            if c.is_whitespace() {
                rest = &rest[c.len_utf8()..];
                continue;
            }
            let unit_len = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
            let unit = &rest[..unit_len];
            let n: f32 = num.parse().ok()?;
            total += match unit {
                "s" | "sec" | "secs" | "second" | "seconds" => n,
                "m" | "min" | "mins" | "minute" | "minutes" => n * 60.,
                _ => return None,
            };
            any = true;
            num.clear();
            rest = &rest[unit_len..];
        }
        if !num.is_empty() {
            // A bare number up to 20 is minutes ("7"); above that it's seconds ("90").
            let n: f32 = num.parse().ok()?;
            total += if n <= 20. { n * 60. } else { n };
            any = true;
        }
        if !any {
            return None;
        }
        total.round() as u32
    };
    (secs > 0).then(|| secs.clamp(LENGTH_RANGE.0, LENGTH_RANGE.1))
}

/// How the video explains the idea.
pub const APPROACHES: [PlanChoice; 6] = [
    choice("Intuitive", "Intuitive", "Pictures first, light on notation", "Build intuition visually before any formula. Keep notation minimal and introduce a symbol only once the picture has made it obvious."),
    choice("Step by step", "Steps", "A worked derivation, one move at a time", "Work through the idea as a sequence of small steps. Show and justify each step on screen before moving to the next."),
    choice("Real-world story", "Story", "Starts from an everyday situation", "Frame the whole video around one concrete everyday situation. Open with it, and come back to it at the end to show the idea solving it."),
    choice("Proof", "Proof", "Why it has to be true", "Explain why the result holds: state it, then give a clear, friendly but rigorous argument with every step visible."),
    choice("Worked examples", "Examples", "Mostly examples, little theory", "Keep the theory brief. Spend most of the time on two or three worked examples of increasing difficulty."),
    choice("Quick recap", "Recap", "Revision of something already taught", "The class has met this before: recap the key ideas and the common mistakes briskly, without a long build-up."),
];

/// Who the video is for. The first entry leaves it to the request.
pub const LEVELS: [PlanChoice; 5] = [
    choice("Any level", "Any", "Inferred from your request", ""),
    choice("Primary", "Primary", "Ages 7 to 11", "Primary school (ages 7 to 11)"),
    choice("Middle school", "Middle", "Ages 11 to 14", "Middle school (ages 11 to 14)"),
    choice("High school", "High", "Ages 14 to 18", "High school (ages 14 to 18)"),
    choice("University", "University", "Undergraduate", "First-year university students"),
];

/// Narration speed: (choice, spoken words per second).
pub const PACES: [(PlanChoice, f32); 3] = [
    (choice("Relaxed", "Relaxed pace", "More time to look at each step", ""), 2.1),
    (choice("Normal", "Normal pace", "A steady classroom pace", ""), 2.5),
    (choice("Brisk", "Brisk pace", "Quicker, for review or older students", ""), 2.9),
];

/// How the video ends.
pub const ENDINGS: [PlanChoice; 4] = [
    choice("Practice question", "Question", "One question for the class to try", "End with one practice question for the class to try."),
    choice("Recap", "Recap", "A short summary of the key points", "End with a short recap of the key points. No practice question."),
    choice("Recap and question", "Both", "Summary, then a question to try", "End with a short recap of the key points, then one practice question for the class to try."),
    choice("No wrap-up", "None", "Finish on the last idea", "Finish on the final idea itself. No recap and no practice question."),
];


pub fn format_time(secs: f32) -> String {
    let s = secs.max(0.) as u32;
    format!("{:02}:{:02}", s / 60, s % 60)
}

pub fn format_short(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// "1.9k" style token counts.
pub fn format_tokens(n: u64) -> String {
    if n >= 10_000 {
        format!("{}k", n / 1000)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod length_tests {
    use super::*;

    #[test]
    fn typed_lengths() {
        assert_eq!(parse_length("3:30"), Some(210));
        assert_eq!(parse_length("7"), Some(420));
        assert_eq!(parse_length("7 min"), Some(420));
        assert_eq!(parse_length("90s"), Some(90));
        assert_eq!(parse_length("1.5 m"), Some(90));
        assert_eq!(parse_length("2m 30s"), Some(150));
        assert_eq!(parse_length("45 seconds"), Some(45));
        assert_eq!(parse_length("5s"), Some(15));
        assert_eq!(parse_length("90"), Some(90));
        assert_eq!(parse_length("10"), Some(600));
        assert_eq!(parse_length("soon"), None);
        assert_eq!(parse_length(""), None);
        // Non-ASCII spaces (IME, pasted) must not split a character.
        assert_eq!(parse_length("5　min"), Some(300));
        assert_eq!(parse_length("5 min"), Some(300));
        assert_eq!(parse_length("0"), None);
    }

    #[test]
    fn labels() {
        assert_eq!(length_label(45), "45 s");
        assert_eq!(length_label(600), "10 min");
        assert_eq!(length_label(210), "3:30");
    }
}

#[cfg(test)]
mod style_tests {
    /// The pickers' names and order must match the engine themes they select.
    #[test]
    fn house_styles_match_engine_styles() {
        let engine = studio_render::style::STYLES;
        assert_eq!(super::HOUSE_STYLES.len(), engine.len());
        for (a, b) in super::HOUSE_STYLES.iter().zip(engine.iter()) {
            assert_eq!(a.name, b.name);
        }
    }
}
