//! Bundled assets: fonts, icons and placeholder scene art.

use gpui::{App, AssetSource, Result, SharedString};
use rust_embed::RustEmbed;
use std::borrow::Cow;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/*.svg"]
#[include = "art/*.svg"]
#[include = "fonts/*.ttf"]
#[include = "lesson-fonts/*.ttf"]
#[include = "brand/*.svg"]
pub struct Assets;
// `lesson-fonts/` holds the house-style fonts used inside videos (all SIL OFL, licences
// alongside). The engine registers them; the app UI only loads `fonts/`.

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(|f| f.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

/// Register the UI fonts with GPUI's text system. Call once at startup.
pub fn load_fonts(cx: &mut App) -> Result<()> {
    let fonts = Assets::iter()
        .filter(|p| p.starts_with("fonts/") && p.ends_with(".ttf"))
        .filter_map(|p| Assets::get(&p).map(|f| f.data))
        .collect::<Vec<_>>();
    cx.text_system().add_fonts(fonts)
}

/// The full-colour app mark (rendered as an image, not a mask).
pub const LOGO: &str = "brand/logo.svg";

/// Placeholder lesson frames. Phase 3 replaces these with engine frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Art {
    Circle,
    Intro,
    Graph,
    Eq,
    Quiz,
    Photo,
    Vec,
    Frac,
}

impl Art {
    pub const ALL: [Art; 8] = [
        Art::Circle,
        Art::Intro,
        Art::Graph,
        Art::Eq,
        Art::Quiz,
        Art::Photo,
        Art::Vec,
        Art::Frac,
    ];

    pub fn path(self) -> &'static str {
        match self {
            Art::Circle => "art/circle.svg",
            Art::Intro => "art/intro.svg",
            Art::Graph => "art/graph.svg",
            Art::Eq => "art/eq.svg",
            Art::Quiz => "art/quiz.svg",
            Art::Photo => "art/photo.svg",
            Art::Vec => "art/vec.svg",
            Art::Frac => "art/frac.svg",
        }
    }
}
