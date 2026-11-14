//! Keyboard focus for stateless controls (buttons, toggles, segmented tabs).
//!
//! A control keeps a focus handle in element state keyed by its id, joins the Tab order,
//! and shows a ring only when focused from the keyboard (`focus_visible_style`). GPUI turns
//! Enter and Space on a focused element into its `on_click`. A mouse click gives the focus
//! back afterwards, so Space keeps playing the video instead of pressing that control again.

use crate::theme::ActiveTheme;
use gpui::{App, ClickEvent, Div, ElementId, FocusHandle, InteractiveElement, Stateful, Styled, Window};

/// Key context set on focusable controls, so single-key shortcuts step aside while one
/// has keyboard focus.
pub const CONTEXT: &str = "Control";

/// The focus handle for the control `id`, kept for as long as the control is drawn.
pub fn handle(id: &ElementId, window: &mut Window, cx: &mut App) -> FocusHandle {
    let key = ElementId::Name(format!("focus:{id}").into());
    window.use_keyed_state(key, cx, |_, cx| cx.focus_handle()).read(cx).clone()
}

/// Make a control focusable with Tab and give it a keyboard-only focus ring. A disabled
/// control is skipped by Tab.
pub fn focusable(el: Stateful<Div>, h: &FocusHandle, enabled: bool, cx: &App) -> Stateful<Div> {
    let ring = cx.theme().ring;
    let h = h.clone().tab_index(0).tab_stop(enabled);
    el.track_focus(&h).key_context(CONTEXT).focus_visible_style(move |s| s.border_color(ring))
}

/// After a control's click: a mouse click doesn't keep the focus (keyboard clicks do).
pub fn release_after_mouse(e: &ClickEvent, h: &FocusHandle, window: &mut Window, cx: &mut App) {
    if matches!(e, ClickEvent::Mouse(_)) && h.is_focused(window) {
        window.blur(cx);
    }
}
