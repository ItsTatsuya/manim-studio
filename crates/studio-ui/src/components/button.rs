use crate::icon::{Icon, IconName};
use crate::theme::{ActiveTheme, radius, text};
use gpui::{
    AnyElement, ColorExt, App, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px,
};
use std::rc::Rc;

pub type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Variant {
    Primary,
    #[default]
    Secondary,
    Ghost,
    /// Destructive confirm (Delete).
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
    Lg,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<IconName>,
    icon_right: Option<IconName>,
    leading: Option<AnyElement>,
    kbd: Option<SharedString>,
    variant: Variant,
    size: ButtonSize,
    selected: bool,
    disabled: bool,
    full_width: bool,
    tooltip: Option<SharedString>,
    badge: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            label: Some(label.into()),
            ..Self::bare(id)
        }
    }

    /// A square, icon-only button.
    pub fn icon(id: impl Into<ElementId>, icon: IconName) -> Self {
        Self {
            icon: Some(icon),
            variant: Variant::Ghost,
            ..Self::bare(id)
        }
    }

    fn bare(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: None,
            icon_right: None,
            leading: None,
            kbd: None,
            variant: Variant::default(),
            size: ButtonSize::default(),
            selected: false,
            disabled: false,
            full_width: false,
            tooltip: None,
            badge: None,
            on_click: None,
        }
    }

    pub fn primary(mut self) -> Self {
        self.variant = Variant::Primary;
        self
    }

    pub fn danger(mut self) -> Self {
        self.variant = Variant::Danger;
        self
    }

    pub fn ghost(mut self) -> Self {
        self.variant = Variant::Ghost;
        self
    }

    pub fn variant(mut self, v: Variant) -> Self {
        self.variant = v;
        self
    }

    pub fn size(mut self, s: ButtonSize) -> Self {
        self.size = s;
        self
    }

    pub fn small(self) -> Self {
        self.size(ButtonSize::Sm)
    }

    pub fn large(self) -> Self {
        self.size(ButtonSize::Lg)
    }

    pub fn with_icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn icon_right(mut self, icon: IconName) -> Self {
        self.icon_right = Some(icon);
        self
    }

    /// Arbitrary leading element (e.g. a brand mark).
    pub fn leading(mut self, el: impl IntoElement) -> Self {
        self.leading = Some(el.into_any_element());
        self
    }

    pub fn kbd(mut self, k: impl Into<SharedString>) -> Self {
        self.kbd = Some(k.into());
        self
    }

    pub fn selected(mut self, s: bool) -> Self {
        self.selected = s;
        self
    }

    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }

    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Hover label; icon-only buttons should always have one.
    pub fn tooltip(mut self, t: impl Into<SharedString>) -> Self {
        self.tooltip = Some(t.into());
        self
    }

    /// A small count shown after the label (e.g. queue length).
    pub fn badge(mut self, b: impl Into<SharedString>) -> Self {
        self.badge = Some(b.into());
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = crate::components::focus::handle(&self.id, window, cx);
        let enabled = !self.disabled && self.on_click.is_some();
        let t = cx.theme().clone();
        let icon_only = self.label.is_none();
        let (h, pad, radius, font) = match self.size {
            ButtonSize::Sm => (px(28.), px(9.), radius::SEGMENT, text::SMALL),
            ButtonSize::Md => (px(30.), px(12.), radius::CONTROL, text::BODY),
            ButtonSize::Lg => (px(38.), px(16.), radius::CARD, text::LARGE),
        };
        let (bg, fg, hover_bg, hover_fg, border) = match self.variant {
            Variant::Primary => (Some(t.primary), t.on_primary, Some(t.primary.opacity(0.9)), t.on_primary, None),
            Variant::Secondary => (Some(t.surface), t.fg, Some(t.surface_2), t.fg, Some(t.line_strong)),
            // Transparent surface_2, not None: fading from None goes through transparent black.
            Variant::Ghost => (Some(t.surface_2.opacity(0.)), t.fg_2, Some(t.surface_2), t.fg, None),
            Variant::Danger => (Some(t.err), t.on_primary, Some(t.err.opacity(0.88)), t.on_primary, None),
        };
        let bg = if self.selected { Some(t.surface_3) } else { bg };
        let fg = if self.selected { t.fg } else { fg };
        let icon_color = if matches!(self.variant, Variant::Primary | Variant::Danger) { t.on_primary } else { fg };

        let on_click = self.on_click.clone();
        let el = div()
            .id(self.id)
            .transitions(crate::motion::hovers)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(7.))
            .h(h)
            .when(icon_only, |d| d.w(h))
            .when(!icon_only, |d| d.px(pad))
            .when(self.full_width, |d| d.w_full())
            .rounded(radius)
            .text_size(font)
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg)
            .whitespace_nowrap()
            .when_some(bg, |d, bg| d.bg(bg))
            // Every variant has the 1px border box, so the focus ring never shifts layout.
            .border_1()
            .border_color(border.unwrap_or(gpui::transparent_black()))
            .when(self.variant != Variant::Ghost, |d| d.shadow(t.edge()))
            .when(self.disabled, |d| d.opacity(0.45))
            .when(!self.disabled, |d| {
                d.cursor_pointer()
                    .hover_style(move |s| {
                        let s = s.text_color(hover_fg);
                        match hover_bg {
                            Some(bg) if !self.selected => s.bg(bg),
                            _ => s,
                        }
                    })
                    .active_style(|s| s.opacity(0.85))
            })
            .when_some(on_click.filter(|_| !self.disabled), |d, f| {
                let focus = focus.clone();
                d.on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    f(e, w, cx);
                    crate::components::focus::release_after_mouse(e, &focus, w, cx);
                })
            })
            .when_some(self.leading, |d, el| d.child(el))
            .when_some(self.icon, |d, i| {
                d.child(Icon::new(i).size(if icon_only { px(16.) } else { px(14.) }).color(icon_color))
            })
            .when_some(self.label, |d, l| d.child(l))
            .when_some(self.icon_right, |d, i| d.child(Icon::new(i).small().color(icon_color.opacity(0.8))))
            .when_some(self.badge, |d, b| {
                d.child(
                    div()
                        .h(px(18.))
                        .min_w(px(18.))
                        .px(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(radius::THUMB)
                        .bg(t.surface_3)
                        .text_size(text::CAPS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.fg_2)
                        .child(b),
                )
            })
            .when_some(self.kbd, |d, k| d.child(Kbd::new(k).inverse(self.variant == Variant::Primary)))
            .when_some(self.tooltip, |d, tip| d.tooltip(crate::components::Tooltip::text(tip)));
        crate::components::focus::focusable(el, &focus, enabled, cx)
    }
}

/// A keyboard shortcut hint.
#[derive(IntoElement)]
pub struct Kbd {
    keys: SharedString,
    inverse: bool,
}

impl Kbd {
    pub fn new(keys: impl Into<SharedString>) -> Self {
        Self {
            keys: keys.into(),
            inverse: false,
        }
    }

    fn inverse(mut self, i: bool) -> Self {
        self.inverse = i;
        self
    }
}

impl RenderOnce for Kbd {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        let color = if self.inverse { t.on_primary.opacity(0.6) } else { t.fg_3 };
        div()
            .px(px(5.))
            .h(px(18.))
            .flex()
            .items_center()
            .rounded(radius::INNER)
            .border_1()
            .border_color(if self.inverse { color.opacity(0.5) } else { t.line_strong })
            .text_size(text::CAPS)
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            .child(self.keys)
    }
}
