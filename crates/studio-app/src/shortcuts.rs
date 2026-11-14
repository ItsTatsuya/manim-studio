//! Keyboard shortcuts: one table that both binds the keys and feeds the list in
//! Settings, so the two can't drift apart.
//!
//! Single keys (Space, M, arrows...) only fire when no text field has focus, so typing
//! a prompt never plays the video. Modifier shortcuts work everywhere.

use gpui::{App, KeyBinding, actions};

actions!(
    studio,
    [
        RenderVideo,
        NewVideo,
        ToggleSidebar,
        OpenSettings,
        OpenRenders,
        CloseMenus,
        TogglePlay,
        FocusSearch,
        ToggleMute,
        VolumeUp,
        VolumeDown,
        PrevScene,
        NextScene,
        ModeStoryboard,
        ModeStudio,
        ModeCode,
        ToggleCaptions,
        ToggleSafeArea,
        ToggleLoop,
        FocusNext,
        FocusPrev,
        PaletteUp,
        PaletteDown,
    ]
);

/// The workspace root's key context.
pub const CONTEXT: &str = "Workspace";
/// Where single-key shortcuts apply: anywhere in the workspace except a text field or a
/// control focused from the keyboard (there Space and Enter press the control).
const NOT_TYPING: &str = "Workspace && !TextInput && !Control";

/// One row of the shortcuts list in Settings.
pub struct Shortcut {
    pub group: &'static str,
    pub label: &'static str,
    /// GPUI keystroke syntax, e.g. "ctrl-enter".
    pub keys: String,
}

fn table() -> (Vec<KeyBinding>, Vec<Shortcut>) {
    let sec = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
    let mut bindings = Vec::new();
    let mut rows = Vec::new();
    macro_rules! sc {
        ($group:literal, $label:literal, $keys:expr, $action:expr, $ctx:expr) => {{
            let keys: String = $keys;
            bindings.push(KeyBinding::new(&keys, $action, $ctx));
            rows.push(Shortcut { group: $group, label: $label, keys });
        }};
    }

    sc!("Playback", "Play or pause", "space".into(), TogglePlay, Some(NOT_TYPING));
    sc!("Playback", "Previous scene", "left".into(), PrevScene, Some(NOT_TYPING));
    sc!("Playback", "Next scene", "right".into(), NextScene, Some(NOT_TYPING));
    sc!("Playback", "Mute or unmute narration", "m".into(), ToggleMute, Some(NOT_TYPING));
    sc!("Playback", "Volume up", "up".into(), VolumeUp, Some(NOT_TYPING));
    sc!("Playback", "Volume down", "down".into(), VolumeDown, Some(NOT_TYPING));

    sc!("Stage", "Show captions", "c".into(), ToggleCaptions, Some(NOT_TYPING));
    sc!("Stage", "Show the safe area", "g".into(), ToggleSafeArea, Some(NOT_TYPING));
    sc!("Stage", "Loop the scene", "l".into(), ToggleLoop, Some(NOT_TYPING));

    sc!("Views", "Storyboard", format!("{sec}-1"), ModeStoryboard, None);
    sc!("Views", "Studio", format!("{sec}-2"), ModeStudio, None);
    sc!("Views", "Code", format!("{sec}-3"), ModeCode, None);
    sc!("Views", "Show or hide the sidebar", format!("{sec}-b"), ToggleSidebar, None);
    sc!("Views", "Command palette", format!("{sec}-k"), FocusSearch, None);

    sc!("Videos", "New video", format!("{sec}-n"), NewVideo, None);
    sc!("Videos", "Render the video", format!("{sec}-enter"), RenderVideo, None);
    sc!("Videos", "Open renders", format!("{sec}-r"), OpenRenders, None);

    sc!("App", "Open settings", format!("{sec}-,"), OpenSettings, None);
    sc!("App", "Close menus and dialogs", "escape".into(), CloseMenus, None);
    sc!("App", "Next control", "tab".into(), FocusNext, Some(CONTEXT));
    sc!("App", "Previous control", "shift-tab".into(), FocusPrev, Some(CONTEXT));

    // Inside the palette only; not listed (the palette shows its own hints).
    bindings.push(KeyBinding::new("up", PaletteUp, Some("Palette")));
    bindings.push(KeyBinding::new("down", PaletteDown, Some("Palette")));

    (bindings, rows)
}

pub fn bind(cx: &mut App) {
    cx.bind_keys(table().0);
}

/// Every shortcut, in display order.
pub fn list() -> Vec<Shortcut> {
    table().1
}

/// "ctrl-shift-enter" → ["Ctrl", "Shift", "Enter"], for key caps.
pub fn caps(keys: &str) -> Vec<String> {
    // A trailing "-" is the minus key, not a separator.
    let mut parts: Vec<&str> = keys.split('-').collect();
    if keys.ends_with('-') {
        parts.retain(|p| !p.is_empty());
        parts.push("-");
    }
    parts
        .into_iter()
        .map(|k| {
            match k {
                "ctrl" => "Ctrl",
                "cmd" => "⌘",
                "alt" => if cfg!(target_os = "macos") { "⌥" } else { "Alt" },
                "shift" => "Shift",
                "enter" => "Enter",
                "space" => "Space",
                "escape" => "Esc",
                "tab" => "Tab",
                "left" => "←",
                "right" => "→",
                "up" => "↑",
                "down" => "↓",
                other => return other.to_uppercase(),
            }
            .to_string()
        })
        .collect()
}
