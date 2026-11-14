use crate::icon::{Icon, IconName};
use crate::motion;
use crate::theme::{ActiveTheme, num_features, radius, text};
use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px, relative, Animation, AnimationExt,
};
use std::rc::Rc;
use std::time::Duration;

type ValueHandler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Signal,
    Ok,
    Err,
}

impl Tone {
    pub fn colors(self, cx: &App) -> (Hsla, Hsla) {
        let t = cx.theme();
        match self {
            Tone::Neutral => (t.fg_2, t.surface_2),
            Tone::Signal => (t.signal, t.signal_soft),
            Tone::Ok => (t.ok, t.ok_soft),
            Tone::Err => (t.err, t.err_soft),
        }
    }
}

/// A small status label.
#[derive(IntoElement)]
pub struct Chip {
    label: SharedString,
    tone: Tone,
    icon: Option<IconName>,
}

impl Chip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            tone: Tone::Neutral,
            icon: None,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
}

impl RenderOnce for Chip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (fg, bg) = self.tone.colors(cx);
        let line = cx.theme().line;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(22.))
            .px(px(8.))
            .rounded(radius::CHIP)
            .bg(bg)
            .when(self.tone == Tone::Neutral, |d| d.border_1().border_color(line))
            .text_size(text::SMALL)
            .font_weight(FontWeight::MEDIUM)
            .font_features(num_features())
            .text_color(fg)
            .whitespace_nowrap()
            .when_some(self.icon, |d, i| d.child(Icon::new(i).size(px(13.)).color(fg)))
            .child(self.label)
    }
}

/// A status dot. Signal dots carry a soft halo.
#[derive(IntoElement)]
pub struct Dot {
    tone: Tone,
}

impl Dot {
    pub fn new(tone: Tone) -> Self {
        Self { tone }
    }
}

impl RenderOnce for Dot {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (fg, soft) = self.tone.colors(cx);
        let fg = if self.tone == Tone::Neutral { cx.theme().fg_3 } else { fg };
        div()
            .flex_none()
            .size(px(12.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .when(self.tone == Tone::Signal, |d| d.bg(soft))
            .child(div().size(px(6.)).rounded_full().bg(fg))
    }
}

/// Determinate progress, 0.0–1.0.
#[derive(IntoElement)]
pub struct ProgressBar {
    value: f32,
    tone: Tone,
    height: f32,
    /// Glide between values (needs an id unique among bars on screen).
    animate: Option<ElementId>,
    /// No known fraction: a segment sweeps across.
    indeterminate: Option<ElementId>,
}

impl ProgressBar {
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0., 1.),
            tone: Tone::Signal,
            height: 3.,
            animate: None,
            indeterminate: None,
        }
    }

    pub fn animated(mut self, id: impl Into<ElementId>) -> Self {
        self.animate = Some(id.into());
        self
    }

    /// Progress with no known end: a segment sweeps across the track.
    pub fn indeterminate(id: impl Into<ElementId>) -> Self {
        let mut b = Self::new(0.);
        b.indeterminate = Some(id.into());
        b
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
}

impl RenderOnce for ProgressBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        let fill = match self.tone {
            Tone::Neutral => t.fg_2,
            other => other.colors(cx).0,
        };
        let track = div().w_full().h(px(self.height)).rounded_full().bg(t.surface_3).overflow_hidden();
        if let Some(id) = self.indeterminate {
            return track.relative().child(
                div()
                    .absolute()
                    .top_0()
                    .h_full()
                    .w(relative(0.35))
                    .rounded_full()
                    .bg(fill)
                    .with_animation(
                        id,
                        Animation::new(Duration::from_millis(1400)).repeat().with_easing(gpui::ease_in_out),
                        |d, v| d.left(relative(-0.35 + v * 1.35)),
                    ),
            );
        }
        let bar = div().h_full().w(relative(self.value)).rounded_full().bg(fill);
        match self.animate {
            Some(id) => track.child(bar.id(id).transitions(|t| t.w(motion::BASE))),
            None => track.child(bar),
        }
    }
}

/// Three dots breathing in turn: "working on it".
#[derive(IntoElement)]
pub struct Pulse {
    id: ElementId,
    tone: Tone,
}

impl Pulse {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), tone: Tone::Signal }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
}

impl RenderOnce for Pulse {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (fg, _) = self.tone.colors(cx);
        let fg = if self.tone == Tone::Neutral { cx.theme().fg_2 } else { fg };
        let base = self.id.clone();
        div().flex_none().flex().items_center().gap(px(3.)).children((0..3).map(move |i| {
            div().size(px(5.)).rounded_full().bg(fg).with_animation(
                (SharedString::from(format!("{base:?}-{i}")), i as usize),
                Animation::new(Duration::from_millis(1100)).repeat(),
                move |d, v| {
                    // Each dot peaks a third of a cycle after the previous one.
                    let phase = (v - i as f32 * 0.18).rem_euclid(1.);
                    let wave = (phase * std::f32::consts::TAU).sin().max(0.);
                    d.opacity(0.25 + 0.75 * wave)
                },
            )
        }))
    }
}

/// On/off switch.
#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    on: bool,
    on_toggle: Option<ValueHandler<bool>>,
}

impl Toggle {
    pub fn new(id: impl Into<ElementId>, on: bool) -> Self {
        Self {
            id: id.into(),
            on,
            on_toggle: None,
        }
    }

    pub fn on_toggle(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme().clone();
        let on = self.on;
        let f = self.on_toggle.clone();
        let anim_id = self.id.clone();
        let focus = super::focus::handle(&self.id, window, cx);
        let el = super::focus::focusable(div().id(self.id.clone()), &focus, f.is_some(), cx);
        el.flex_none()
            .relative()
            .w(px(32.))
            .h(px(18.))
            .rounded_full()
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .when_some(f, |d, f| {
                d.on_click(move |e: &ClickEvent, w, cx| {
                    f(!on, w, cx);
                    super::focus::release_after_mouse(e, &focus, w, cx);
                })
            })
            .with_spring(anim_id, motion::spring_to(if on { 1. } else { 0. }), move |d, v| {
                let v = v.clamp(0., 1.);
                d.bg(motion::lerp_color(t.surface_3, t.fg, v)).child(
                    div()
                        .absolute()
                        .top(px(1.))
                        .left(px(1. + 14. * v))
                        .size(px(14.))
                        .rounded_full()
                        .bg(motion::lerp_color(t.fg_3, t.window, v)),
                )
            })
    }
}

/// One item in a [`Segmented`] control.
pub struct Segment {
    pub label: SharedString,
    pub icon: Option<IconName>,
}

impl Segment {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            icon: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// Tabs on a recessed track; the selected item is raised.
#[derive(IntoElement)]
pub struct Segmented {
    id: SharedString,
    items: Vec<Segment>,
    selected: usize,
    fill: bool,
    hide_icons: bool,
    on_select: Option<ValueHandler<usize>>,
    /// The item the pointer is moving over.
    on_item_hover: Option<ValueHandler<usize>>,
}

impl Segmented {
    pub fn new(id: impl Into<SharedString>, items: Vec<Segment>, selected: usize) -> Self {
        Self {
            id: id.into(),
            items,
            selected,
            fill: false,
            hide_icons: false,
            on_select: None,
            on_item_hover: None,
        }
    }

    /// Stretch items to the full width.
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }

    /// Called with the item under the pointer as it moves over the items. Mouse-move
    /// based (stopping propagation), because hover state is unreliable inside popovers;
    /// the caller clears its own state from moves elsewhere.
    pub fn on_item_hover(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_item_hover = Some(Rc::new(f));
        self
    }

    pub fn hide_icons(mut self, hide: bool) -> Self {
        self.hide_icons = hide;
        self
    }

    pub fn on_select(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Segmented {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme().clone();
        let id = self.id.clone();
        let n = self.items.len().max(1) as f32;
        let selected = self.selected;
        let fill = self.fill;
        // Where each tab sat last frame, relative to the track: content-sized tabs
        // have different widths, so the pill needs them to slide and resize.
        let spans = window.use_keyed_state((id.clone(), usize::MAX - 1), cx, |_, _| Vec::<(f32, f32)>::new());
        let span = (!fill).then(|| spans.read(cx).get(selected).copied()).flatten();
        let pill = |t: &crate::theme::Theme| {
            div().rounded(radius::SEGMENT).bg(t.raised).border_1().border_color(t.line).shadow(t.edge())
        };
        let indicator: Option<AnyElement> = if fill {
            // Every item is exactly 1/n wide, so one pill slides between them.
            Some(
                div()
                    .absolute()
                    .top(px(3.))
                    .bottom(px(3.))
                    .left(px(3.))
                    .right(px(3.))
                    .child(
                        pill(&t)
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .w(relative(1. / n))
                            .with_spring((id.clone(), usize::MAX), motion::spring_to(selected as f32), move |d, v| {
                                d.left(relative(v / n))
                            }),
                    )
                    .into_any_element(),
            )
        } else {
            span.map(|(x, w)| {
                // Slide to the tab's left edge and stretch to its width.
                div()
                    .absolute()
                    .top(px(3.))
                    .bottom(px(3.))
                    .flex()
                    .child(
                        pill(&t)
                            .h_full()
                            .with_spring((id.clone(), usize::MAX - 2), motion::spring_to(w), |d, w| d.w(px(w))),
                    )
                    .with_spring((id.clone(), usize::MAX - 3), motion::spring_to(x), |d, x| d.left(px(x)))
                    .into_any_element()
            })
        };
        let has_pill = indicator.is_some();
        let measure = {
            let spans = spans.clone();
            move |bounds: Vec<gpui::Bounds<gpui::Pixels>>, _: &mut Window, cx: &mut App| {
                // Children are [indicator?, tabs...]; offsets are from the track's inner edge.
                let tabs = if has_pill { &bounds[1.min(bounds.len())..] } else { &bounds[..] };
                let Some(first) = tabs.first() else { return };
                let x0 = f32::from(first.origin.x) - 3.;
                let next: Vec<(f32, f32)> =
                    tabs.iter().map(|b| (f32::from(b.origin.x) - x0, f32::from(b.size.width))).collect();
                spans.update(cx, |s, cx| {
                    let moved = s.len() != next.len() || s.iter().zip(&next).any(|(a, b)| (a.0 - b.0).abs() > 0.5 || (a.1 - b.1).abs() > 0.5);
                    if moved {
                        *s = next;
                        cx.notify();
                    }
                });
            }
        };
        div()
            .when(!fill, |d| d.on_children_prepainted(measure))
            .id(self.id)
            .relative()
            .flex()
            .flex_none()
            .when(!fill, |d| d.gap(px(2.)))
            .p(px(3.))
            .rounded(radius::CARD)
            .bg(t.surface_2)
            .border_1()
            .border_color(t.line)
            .when(fill, |d| d.w_full())
            .children(indicator)
            .children(self.items.into_iter().enumerate().map(|(i, item)| {
                let on = i == selected;
                let f = self.on_select.clone();
                let item_id: ElementId = (id.clone(), i).into();
                let focus = super::focus::handle(&item_id, window, cx);
                let release = focus.clone();
                super::focus::focusable(div().id(item_id), &focus, f.is_some(), cx)
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .h(px(24.))
                    .px(px(10.))
                    .rounded(radius::SEGMENT)
                    // Same box for every state, so selecting never resizes anything.
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .text_size(text::BODY)
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_nowrap()
                    .when(fill, |d| d.flex_1().min_w_0())
                    .text_color(if on { t.fg } else { t.fg_2 })
                    // Before the tabs are measured (first frame) the selected tab draws its own pill.
                    .when(on && !has_pill, |d| d.bg(t.raised).border_color(t.line).shadow(t.edge()))
                    .when(!on, |d| d.cursor_pointer().hover_style(|s| s.text_color(t.fg)))
                    .when_some(item.icon.filter(|_| !self.hide_icons), |d, ic| {
                        d.child(Icon::new(ic).small().color(if on { t.fg } else { t.fg_2 }))
                    })
                    .child(item.label)
                    .when_some(f, |d, f| {
                        d.on_click(move |e, w, cx| {
                            f(i, w, cx);
                            super::focus::release_after_mouse(e, &release, w, cx);
                        })
                    })
                    .when_some(self.on_item_hover.clone(), |d, f| {
                        d.on_mouse_move(move |_, w, cx| {
                            cx.stop_propagation();
                            f(i, w, cx)
                        })
                    })
            }))
    }
}

/// Uppercase section label.
pub fn caps(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(text::CAPS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().fg_3)
        .child(label.into().to_uppercase())
}

/// The standard elevated panel.
pub fn panel(cx: &App) -> Div {
    let t = cx.theme();
    div()
        .flex()
        .flex_col()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .rounded(radius::PANEL)
        .bg(t.surface)
        .border_1()
        .border_color(t.line)
        .shadow(t.edge())
}

/// A floating layer (transport, popover, toast).
pub fn floating(cx: &App) -> Div {
    let t = cx.theme();
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(4.))
        .rounded(radius::CARD)
        .bg(t.raised)
        .border_1()
        .border_color(t.line_strong)
        .shadow(t.pop_shadow())
}

/// A brand mark tile for an assistant provider.
pub fn provider_mark(icon: IconName, size: f32, cx: &App) -> AnyElement {
    let t = cx.theme();
    let color = if icon == IconName::Claude { t.claude } else { t.fg };
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.25))
        .bg(t.surface_3)
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(icon).size(px(size * 0.55)).color(color))
        .into_any_element()
}
