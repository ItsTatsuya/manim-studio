//! Manim Studio's design system for GPUI-CE.
//!
//! - [`theme`]: colour, type, space and radius tokens (DESIGN.md §4)
//! - [`assets`]: embedded fonts, icons and placeholder art
//! - [`components`]: buttons, chips, segmented tabs, menus, text input, …

pub mod assets;
pub mod components;
pub mod icon;
pub mod motion;
pub mod theme;

pub use gpui;
pub use icon::{Icon, IconName};
pub use theme::{ActiveTheme, Theme, ThemeMode};

/// Register fonts and key bindings. Call once from `Application::run`.
pub fn init(cx: &mut gpui::App) -> anyhow::Result<()> {
    assets::load_fonts(cx)?;
    components::text_input::bind_keys(cx);
    Ok(())
}
