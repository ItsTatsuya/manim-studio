//! User preferences, persisted as JSON in the platform config directory.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use studio_ui::theme::Palette;
use studio_ui::ThemeMode;

use crate::model::{Reasoning, Tier};

/// The default watermark for exported videos (projects can turn it off).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatermarkPrefs {
    /// 0 off, 1 text, 2 image.
    pub mode: usize,
    pub text: String,
    /// A copy in the data folder (`watermark/`), so moving the original doesn't break it.
    pub image: Option<PathBuf>,
    /// 0..9, row by row from the top left (8 = bottom right).
    pub anchor: usize,
    /// Size relative to the default (1.0).
    pub scale: f32,
    /// Opacity 0.1..=1.
    pub alpha: f32,
    /// Gap to the edges as a fraction of the video's shorter side.
    pub margin: f32,
    /// Index into `engine::WM_COLORS` (text only).
    pub color: usize,
    /// Soft shadow behind text.
    pub shadow: bool,
}

impl Default for WatermarkPrefs {
    fn default() -> Self {
        Self { mode: 0, text: String::new(), image: None, anchor: 8, scale: 1.0, alpha: 0.6, margin: 0.035, color: 0, shadow: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeMode,
    /// Colour family of the app chrome.
    pub palette: Palette,
    pub onboarded: bool,
    pub sidebar_collapsed: bool,
    pub auto_switch_provider: bool,
    pub lean_context: bool,
    pub repair_tries: u8,
    pub planning_tier: Tier,
    pub planning_reasoning: Reasoning,
    pub build_tier: Tier,
    pub build_reasoning: Reasoning,
    /// Raise reasoning one step each time a repair fails, then drop back.
    pub escalate_repairs: bool,
    /// Index into `TURN_BUDGETS`.
    pub turn_budget: usize,
    pub show_usage: bool,
    pub house_style: usize,
    pub auto_render: bool,
    pub update_channel: usize,
    /// The assistant new videos use ("claude" / "codex").
    pub default_provider: String,
    /// Index into `engine::OUTPUTS`.
    pub output: usize,
    /// Index into `engine::BACKENDS`.
    pub renderer: usize,
    /// Index into `engine::ENCODERS`.
    pub encoder: usize,
    /// New-video options: indexes into `model::LENGTHS`, `APPROACHES`, `LEVELS`, `PACES`, `ENDINGS`.
    /// Target length in seconds.
    pub video_seconds: u32,
    pub approach: usize,
    pub level: usize,
    pub pace: usize,
    pub ending: usize,
    /// Last plan usage each assistant reported (shown until the next turn reports).
    pub usage: Vec<UsageRecord>,
    /// The video folder open when Studio last closed.
    pub last_video: Option<std::path::PathBuf>,
    /// Sidebar folder order set by dragging (project folders). Empty: newest first.
    pub folder_order: Vec<std::path::PathBuf>,
    /// Speak the narration (system voice).
    pub narration: bool,
    /// Preview playback volume, 0..=1.
    pub volume: f32,
    /// Preview playback muted (the volume is kept for unmuting).
    pub muted: bool,
    /// Voice name; the system default when `None`.
    pub voice: Option<String>,
    /// Index into `engine::SPEEDS`.
    pub speed: usize,
    pub watermark: WatermarkPrefs,
    /// Natural voice model: index into `studio_render::kokoro::Quality::ALL` (Standard first).
    pub voice_model: usize,
    /// 0 off, 1 .srt file, 2 burned in + .srt.
    pub captions: usize,
    /// Last update check (Unix seconds).
    pub last_update_check: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageRecord {
    /// "claude" / "codex"
    pub provider: String,
    pub window: Option<f32>,
    pub week: Option<f32>,
    pub resets_at: Option<u64>,
    pub week_resets_at: Option<u64>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            palette: Palette::Graphite,
            onboarded: false,
            sidebar_collapsed: false,
            auto_switch_provider: true,
            lean_context: true,
            repair_tries: 3,
            planning_tier: Tier::Fast,
            planning_reasoning: Reasoning::Auto,
            build_tier: Tier::Balanced,
            build_reasoning: Reasoning::Auto,
            escalate_repairs: true,
            turn_budget: 1,
            show_usage: true,
            house_style: 0,
            auto_render: true,
            update_channel: 0,
            default_provider: String::new(),
            output: 0,
            renderer: 0,
            encoder: 0,
            video_seconds: 120,
            approach: 0,
            level: 0,
            pace: 1,
            ending: 0,
            usage: Vec::new(),
            last_video: None,
            folder_order: Vec::new(),
            narration: true,
            volume: 1.0,
            muted: false,
            voice: None,
            speed: 1,
            watermark: WatermarkPrefs::default(),
            voice_model: 0,
            captions: 1,
            last_update_check: None,
        }
    }
}

fn path() -> Option<PathBuf> {
    if let Some(home) = crate::store::studio_home() {
        return Some(home.join("config").join("settings.json"));
    }
    directories::ProjectDirs::from("app", "ManimStudio", "Manim Studio")
        .map(|d| d.config_dir().join("settings.json"))
}

impl Prefs {
    /// True when every assistant/token setting matches the recommended defaults.
    pub fn ai_is_recommended(&self) -> bool {
        let d = Self::default();
        self.planning_tier == d.planning_tier
            && self.planning_reasoning == d.planning_reasoning
            && self.build_tier == d.build_tier
            && self.build_reasoning == d.build_reasoning
            && self.escalate_repairs == d.escalate_repairs
            && self.turn_budget == d.turn_budget
            && self.lean_context == d.lean_context
            && self.repair_tries == d.repair_tries
    }

    /// Restore the assistant/token settings; leaves appearance and everything else alone.
    pub fn reset_ai(&mut self) {
        let d = Self::default();
        self.planning_tier = d.planning_tier;
        self.planning_reasoning = d.planning_reasoning;
        self.build_tier = d.build_tier;
        self.build_reasoning = d.build_reasoning;
        self.escalate_repairs = d.escalate_repairs;
        self.turn_budget = d.turn_budget;
        self.lean_context = d.lean_context;
        self.repair_tries = d.repair_tries;
    }

    pub fn usage_for(&self, p: crate::model::Provider) -> Option<&UsageRecord> {
        self.usage.iter().find(|u| u.provider == p.key())
    }

    pub fn remember_usage(&mut self, p: crate::model::Provider, rl: &studio_agent::events::RateLimits) {
        let rec = match self.usage.iter_mut().find(|u| u.provider == p.key()) {
            Some(r) => r,
            None => {
                self.usage.push(UsageRecord { provider: p.key().into(), ..Default::default() });
                self.usage.last_mut().unwrap()
            }
        };
        if rl.window.is_some() {
            rec.window = rl.window;
        }
        if rl.week.is_some() {
            rec.week = rl.week;
        }
        if rl.window_resets_at.is_some() {
            rec.resets_at = rl.window_resets_at;
        }
        if rl.week_resets_at.is_some() {
            rec.week_resets_at = rl.week_resets_at;
        }
    }

    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Prefs>(&b).ok())
            .unwrap_or_default()
            .sanitized()
    }

    /// Indexes into fixed lists clamped, so a hand-edited or older file can't panic.
    fn sanitized(mut self) -> Self {
        self.house_style = self.house_style.min(studio_render::style::STYLES.len() - 1);
        self
    }

    /// Best-effort save; preferences are never worth crashing over.
    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }
}
