//! Design tokens. Values mirror `design/DESIGN.md` §4 and the CSS variables in
//! `design/mockups/index.html`; change them together.

use gpui::{
    App, BoxShadow, FontFeatures, Global, Hsla, Pixels, Window, WindowAppearance, point, px, rgb,
    rgb_to_hsla, rgba,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

/// Which palette to use. `System` follows the OS appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => "Match system",
            ThemeMode::Light => "Light",
            ThemeMode::Dark => "Dark",
        }
    }
}

/// The colour family of the app chrome. Each has a light and a dark version: the
/// neutrals take a faint tint of the family's hue and the accent (`signal`) changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Palette {
    #[default]
    Graphite,
    Frost,
    Ocean,
    Moss,
    Dusk,
    Sand,
}

impl Palette {
    pub const ALL: [Palette; 6] = [Palette::Graphite, Palette::Frost, Palette::Ocean, Palette::Moss, Palette::Dusk, Palette::Sand];

    pub fn label(self) -> &'static str {
        match self {
            Palette::Graphite => "Graphite",
            Palette::Frost => "Frost",
            Palette::Ocean => "Ocean",
            Palette::Moss => "Moss",
            Palette::Dusk => "Dusk",
            Palette::Sand => "Sand",
        }
    }

    /// Hue (0–1) and saturation of the neutrals for dark and light; `None` keeps them grey.
    fn tint(self) -> Option<(f32, f32, f32)> {
        match self {
            Palette::Graphite => None,
            Palette::Frost => Some((0.60, 0.24, 0.20)),
            Palette::Ocean => Some((0.52, 0.24, 0.18)),
            Palette::Moss => Some((0.36, 0.14, 0.12)),
            Palette::Dusk => Some((0.71, 0.22, 0.16)),
            Palette::Sand => Some((0.09, 0.16, 0.22)),
        }
    }

    /// The accent for dark and light.
    fn signal(self) -> (u32, u32) {
        match self {
            Palette::Graphite => (0xf0b35a, 0xc77d12),
            Palette::Frost => (0x82aef5, 0x2f64c8),
            Palette::Ocean => (0x5fd0c6, 0x0e7c78),
            Palette::Moss => (0xe2c36b, 0x8f6f10),
            Palette::Dusk => (0xb9a0f7, 0x6a4bc8),
            Palette::Sand => (0xe9a274, 0xb35a2a),
        }
    }

    /// What the swatch in Settings shows: (background, accent) in the given appearance.
    pub fn swatch(self, appearance: Appearance) -> (Hsla, Hsla) {
        let t = Theme::build(self, appearance);
        (t.surface, t.signal)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub appearance: Appearance,
    pub window: Hsla,
    pub sidebar: Hsla,
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub surface_3: Hsla,
    pub raised: Hsla,
    pub stage: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub fg: Hsla,
    pub fg_2: Hsla,
    pub fg_3: Hsla,
    pub primary: Hsla,
    pub on_primary: Hsla,
    pub signal: Hsla,
    pub signal_soft: Hsla,
    pub ok: Hsla,
    pub ok_soft: Hsla,
    pub err: Hsla,
    pub err_soft: Hsla,
    pub ring: Hsla,
    pub highlight: Hsla,
    pub shadow: Hsla,
    pub window_close_hover: Hsla,
    /// Claude's brand orange. Used only for the provider mark.
    pub claude: Hsla,
}

fn hex(v: u32) -> Hsla {
    rgb_to_hsla(rgb(v))
}

fn hexa(v: u32) -> Hsla {
    rgb_to_hsla(rgba(v))
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            appearance: Appearance::Dark,
            window: hex(0x111113),
            sidebar: hex(0x111113),
            surface: hex(0x1b1b1e),
            surface_2: hex(0x222226),
            surface_3: hex(0x2c2c31),
            raised: hex(0x29292d),
            stage: hex(0x0c0c0e),
            line: hexa(0xffffff0b),
            line_strong: hexa(0xffffff16),
            fg: hex(0xededf0),
            fg_2: hex(0xa5a5ad),
            fg_3: hex(0x87878f),
            primary: hex(0xe8e8ec),
            on_primary: hex(0x1a1a1d),
            signal: hex(0xf0b35a),
            signal_soft: hexa(0xf0b35a1c),
            ok: hex(0x83c89b),
            ok_soft: hexa(0x83c89b1a),
            err: hex(0xed8588),
            err_soft: hexa(0xed85881a),
            ring: hexa(0xaaaab5aa),
            highlight: hexa(0xffffff0a),
            shadow: hexa(0x000000a0),
            window_close_hover: hex(0xc42b1c),
            claude: hex(0xd97757),
        }
    }

    pub fn light() -> Self {
        Self {
            appearance: Appearance::Light,
            window: hex(0xececee),
            sidebar: hex(0xececee),
            surface: hex(0xf7f7f8),
            surface_2: hex(0xefeff1),
            surface_3: hex(0xe4e4e8),
            raised: hex(0xffffff),
            stage: hex(0xdcdce0),
            line: hexa(0x03030212),
            line_strong: hexa(0x03030220),
            fg: hex(0x232327),
            fg_2: hex(0x4f4f57),
            fg_3: hex(0x6b6b74),
            primary: hex(0x27272b),
            on_primary: hex(0xf7f7f8),
            signal: hex(0xc77d12),
            signal_soft: hexa(0xc77d1218),
            ok: hex(0x24724c),
            ok_soft: hexa(0x24724c18),
            err: hex(0xb83842),
            err_soft: hexa(0xb8384216),
            ring: hexa(0x777782aa),
            highlight: hexa(0xffffffb0),
            shadow: hexa(0x0000002a),
            window_close_hover: hex(0xc42b1c),
            claude: hex(0xd97757),
        }
    }

    pub fn for_appearance(appearance: Appearance) -> Self {
        Self::build(Palette::Graphite, appearance)
    }

    /// The theme for a palette in an appearance.
    pub fn build(palette: Palette, appearance: Appearance) -> Self {
        let mut t = match appearance {
            Appearance::Dark => Self::dark(),
            Appearance::Light => Self::light(),
        };
        let dark = appearance == Appearance::Dark;
        if let Some((h, sd, sl)) = palette.tint() {
            let s = if dark { sd } else { sl };
            // Text gets a third of the tint so it stays neutral to read.
            let tint = |mut c: Hsla, k: f32| {
                c.color.hue = (h * 360.0).into();
                c.color.saturation = s * k;
                c
            };
            for c in [&mut t.window, &mut t.sidebar, &mut t.surface, &mut t.surface_2, &mut t.surface_3, &mut t.raised, &mut t.stage] {
                *c = tint(*c, 1.0);
            }
            for c in [&mut t.fg, &mut t.fg_2, &mut t.fg_3, &mut t.primary, &mut t.on_primary] {
                *c = tint(*c, 0.35);
            }
            t.ring = tint(t.ring, 1.5);
        }
        let (sd, sl) = palette.signal();
        let signal = hex(if dark { sd } else { sl });
        t.signal = signal;
        let mut soft = signal;
        soft.alpha = if dark { 0.11 } else { 0.094 };
        t.signal_soft = soft;
        t
    }

    pub fn is_dark(&self) -> bool {
        self.appearance == Appearance::Dark
    }

    /// Edge treatment for panels and controls: a 1px top highlight.
    /// The hairline ring is a regular border so it stays pixel-crisp.
    pub fn edge(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: self.highlight.into(),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
            inset: true,
        }]
    }

    /// Floating layers only: transport, popovers, toasts.
    pub fn pop_shadow(&self) -> Vec<BoxShadow> {
        vec![
            BoxShadow {
                color: self.shadow.into(),
                offset: point(px(0.), px(16.)),
                blur_radius: px(40.),
                spread_radius: px(-8.),
                inset: false,
            },
            BoxShadow {
                color: self.highlight.into(),
                offset: point(px(0.), px(1.)),
                blur_radius: px(0.),
                spread_radius: px(0.),
                inset: true,
            },
        ]
    }
}

impl Global for Theme {}

/// The user's theme preference, stored next to the resolved [`Theme`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ThemeSettings {
    pub mode: ThemeMode,
    pub palette: Palette,
}

impl Global for ThemeSettings {}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}

/// Resolve a mode against the window's current OS appearance.
pub fn resolve(mode: ThemeMode, os: WindowAppearance) -> Appearance {
    match mode {
        ThemeMode::Light => Appearance::Light,
        ThemeMode::Dark => Appearance::Dark,
        ThemeMode::System => match os {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        },
    }
}

/// Install or refresh the global theme for `mode` and `palette`.
pub fn apply(mode: ThemeMode, palette: Palette, window: &Window, cx: &mut App) {
    let appearance = resolve(mode, window.appearance());
    let changed = !cx.has_global::<ThemeSettings>() || cx.global::<ThemeSettings>().palette != palette;
    cx.set_global(ThemeSettings { mode, palette });
    let needs_swap = changed || !cx.has_global::<Theme>() || cx.global::<Theme>().appearance != appearance;
    if needs_swap {
        cx.set_global(Theme::build(palette, appearance));
    }
}

// ----------------------------------------------------------------------------
// Typography
// ----------------------------------------------------------------------------

pub const FONT_UI: &str = "Inter";
/// Large headings use the same text cut: Inter Display is noticeably narrower
/// and read as cramped under Windows' grayscale antialiasing.
pub const FONT_DISPLAY: &str = "Inter";
pub const FONT_MONO: &str = "JetBrains Mono";

pub mod text {
    use gpui::{Pixels, px};
    pub const DISPLAY: Pixels = px(28.);
    pub const TITLE: Pixels = px(16.);
    pub const BODY: Pixels = px(14.);
    pub const SMALL: Pixels = px(12.5);
    pub const CAPS: Pixels = px(11.);
    /// Large buttons.
    pub const LARGE: Pixels = px(15.);
    /// Body line height.
    pub const LEADING: Pixels = px(20.);
    /// Inter is tight at UI sizes without hinting; a hair of tracking keeps
    /// letters from touching. Inherited by everything under the root.
    pub const TRACKING: Pixels = px(0.15);
}

fn features(tags: &[(&str, u32)]) -> FontFeatures {
    FontFeatures(Arc::new(
        tags.iter().map(|(t, v)| ((*t).to_string(), *v)).collect(),
    ))
}

/// Inter with its default glyphs (no stylistic sets), kerning and contextual alternates.
pub fn ui_features() -> FontFeatures {
    static F: OnceLock<FontFeatures> = OnceLock::new();
    F.get_or_init(|| features(&[("kern", 1), ("calt", 1)])).clone()
}

/// Tabular numerals for times, sizes, percentages and counts.
pub fn num_features() -> FontFeatures {
    static F: OnceLock<FontFeatures> = OnceLock::new();
    F.get_or_init(|| features(&[("kern", 1), ("calt", 1), ("tnum", 1)]))
        .clone()
}

// ----------------------------------------------------------------------------
// Space and radius (4px grid)
// ----------------------------------------------------------------------------

pub mod radius {
    use gpui::{Pixels, px};
    pub const PANEL: Pixels = px(12.);
    pub const COMPOSER: Pixels = px(14.);
    pub const CONTROL: Pixels = px(8.);
    pub const SEGMENT: Pixels = px(6.);
    pub const CHIP: Pixels = px(6.);
    pub const THUMB: Pixels = px(5.);
    /// Menus, popovers, large buttons and stage wells.
    pub const CARD: Pixels = px(10.);
    /// Small marks inside other surfaces (tiny thumbnails, key caps).
    pub const INNER: Pixels = px(4.);
    /// Hairline bars (progress, sliders).
    pub const BAR: Pixels = px(2.);
}

pub mod space {
    use gpui::{Pixels, px};
    pub const GUTTER: Pixels = px(10.);
    pub const ROW: Pixels = px(30.);
    pub const CONTROL: Pixels = px(30.);
    pub const TITLEBAR: Pixels = px(46.);
    pub const SIDEBAR: Pixels = px(236.);
    pub const RIGHT_PANEL: Pixels = px(372.);
}

/// Width below which the sidebar auto-collapses (DESIGN.md §3).
pub const COMPACT_WIDTH: Pixels = px(1240.);

pub fn is_compact(window: &Window) -> bool {
    window.viewport_size().width < COMPACT_WIDTH
}
