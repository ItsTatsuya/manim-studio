//! Shared motion tokens and helpers. Every transition in the app goes through
//! here so timings stay consistent. GPUI skips these automatically when the
//! system asks for reduced motion.

use gpui::{
    Animation, AnimationElement, AnimationExt, ElementId, Hsla, InteractiveElement, IntoElement,
    Interpolate, MotionDurationExt, SpringAnimation, SpringConfig, StyleTransitions, Styled, ease_out_quint, px,
    spring,
};
use std::time::Duration;

/// Hover and press feedback on controls.
pub const HOVER: Duration = Duration::from_millis(120);
/// Popovers, tooltips and small reveals.
pub const FAST: Duration = Duration::from_millis(140);
/// Content swaps: tabs, pages, screens.
pub const BASE: Duration = Duration::from_millis(200);
/// Toasts and larger surfaces.
pub const SLOW: Duration = Duration::from_millis(280);

/// Snappy, barely-overshooting spring for toggles and sliding indicators.
pub const SNAPPY: SpringConfig = spring(520., 38., 1.);

pub fn ease(d: Duration) -> Animation {
    Animation::new(d).with_easing(ease_out_quint())
}

/// Spring that retargets smoothly when `target` changes between renders.
/// Pair with a stable element id.
pub fn spring_to(target: f32) -> SpringAnimation<f32> {
    SpringAnimation::new(SNAPPY).to(target)
}

/// Fade in while rising `rise` px. Use a fresh `id` (for example one that
/// includes the selected tab) to replay it when content changes.
pub fn enter<E: IntoElement + Styled + 'static>(
    el: E,
    id: impl Into<ElementId>,
    duration: Duration,
    rise: f32,
) -> AnimationElement<E> {
    el.with_animation(id, ease(duration), move |el, d| {
        el.opacity(d).relative().top(px((1. - d) * rise))
    })
}

/// Slide in a few px from the anchor edge, for popovers (menus, tooltips, cards).
/// Opaque from the first frame: a fading dark popover over the bright stage reads
/// as a light flash.
pub fn pop<E: IntoElement + Styled + 'static>(
    el: E,
    id: impl Into<ElementId>,
    upward: bool,
) -> AnimationElement<E> {
    let dir = if upward { 1. } else { -1. };
    el.with_animation(id, ease(FAST), move |el, d| el.relative().top(px((1. - d) * 5. * dir)))
}

/// Fade colour and opacity changes (hover, press, selection) instead of snapping.
/// Give repeated elements their own id, or they share one transition.
pub fn hover_fx<E: InteractiveElement>(el: E) -> E {
    el.transitions(hovers)
}

/// The transition set behind `hover_fx`, for use inside builder chains:
/// `.transitions(motion::hovers)`.
///
/// Opacity is left out on purpose: an unset opacity transitions as 0, so an element
/// whose opacity goes from set (pressed, 0.85) back to unset fades out completely and
/// then snaps back. Use `fades` where the element always sets an explicit opacity.
pub fn hovers(t: StyleTransitions) -> StyleTransitions {
    let m = HOVER.with_easing(ease_out_quint());
    t.bg(m.clone()).text_color(m.clone()).border_color(m)
}

/// `hovers` plus opacity. Only for elements that always set `.opacity(..)` explicitly.
pub fn fades(t: StyleTransitions) -> StyleTransitions {
    hovers(t).opacity(HOVER.with_easing(ease_out_quint()))
}

/// Animate width changes (progress fills), so jumps glide.
pub fn width_fx<E: InteractiveElement>(el: E) -> E {
    el.transitions(|t| t.w(BASE.with_easing(ease_out_quint())))
}

/// Fade and grow in from slightly smaller, for dialogs and cards.
pub fn rise<E: IntoElement + Styled + 'static>(
    el: E,
    id: impl Into<ElementId>,
) -> AnimationElement<E> {
    el.with_animation(id, ease(SLOW), move |el, d| {
        el.opacity(d).relative().top(px((1. - d) * 10.))
    })
}

/// Fade in only (backdrops, content that must not move).
pub fn fade<E: IntoElement + Styled + 'static>(
    el: E,
    id: impl Into<ElementId>,
    duration: Duration,
) -> AnimationElement<E> {
    el.with_animation(id, ease(duration), move |el, d| el.opacity(d))
}

pub fn lerp_color(a: Hsla, b: Hsla, t: f32) -> Hsla {
    Interpolate::interpolate(a, b, t.clamp(0., 1.))
}
