use crate::components::button::Button;
use crate::icon::{Icon, IconName};
use crate::theme::{ActiveTheme, radius, text};
use gpui::{
    ColorExt,
    AnyElement, Anchor, App, FontWeight, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, anchored,
    deferred, div, prelude::FluentBuilder, px,
};
use std::rc::Rc;

pub struct MenuItem {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub icon: Option<IconName>,
    /// Destructive (delete): drawn in the error colour.
    pub danger: bool,
    /// Draw a hairline above this item.
    pub divider: bool,
    /// A non-clickable caption ("Move to").
    pub heading: bool,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            detail: None,
            icon: None,
            danger: false,
            divider: false,
            heading: false,
        }
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn divider(mut self) -> Self {
        self.divider = true;
        self
    }

    pub fn heading(label: impl Into<SharedString>) -> Self {
        let mut m = Self::new(label);
        m.heading = true;
        m
    }

    pub fn detail(mut self, d: impl Into<SharedString>) -> Self {
        self.detail = Some(d.into());
        self
    }

    pub fn icon(mut self, i: IconName) -> Self {
        self.icon = Some(i);
        self
    }
}

type Handler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

/// A trigger button with an anchored popover list. The owner keeps `open`
/// state so only one menu is open at a time.
#[derive(IntoElement)]
pub struct Dropdown {
    id: SharedString,
    trigger: Button,
    items: Vec<MenuItem>,
    selected: Option<usize>,
    open: bool,
    upward: bool,
    width: f32,
    on_toggle: Option<Handler<bool>>,
    on_select: Option<Handler<usize>>,
}

impl Dropdown {
    pub fn new(id: impl Into<SharedString>, trigger: Button, items: Vec<MenuItem>) -> Self {
        Self {
            id: id.into(),
            trigger,
            items,
            selected: None,
            open: false,
            upward: false,
            width: 220.,
            on_toggle: None,
            on_select: None,
        }
    }

    pub fn selected(mut self, i: usize) -> Self {
        self.selected = Some(i);
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Open above the trigger (for controls near the bottom of a panel).
    pub fn upward(mut self) -> Self {
        self.upward = true;
        self
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }

    pub fn on_toggle(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(f));
        self
    }

    pub fn on_select(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Dropdown {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme().clone();
        let open = self.open;
        let toggle = self.on_toggle.clone();
        let trigger = {
            let toggle = toggle.clone();
            self.trigger
                .selected(open)
                .on_click(move |_, w, cx| {
                    if let Some(f) = &toggle {
                        f(!open, w, cx)
                    }
                })
        };
        let id = self.id.clone();
        let upward = self.upward;
        let menu: Option<AnyElement> = open.then(|| {
            let close = toggle.clone();
            let list = div()
                .id(SharedString::from(format!("{id}-menu")))
                .occlude()
                .w(px(self.width))
                .p(px(4.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .rounded(radius::CARD)
                // Panel colour, like the new-video options, so menus read as one family.
                .bg(t.surface)
                .border_1()
                .border_color(t.line_strong)
                .shadow(t.pop_shadow())
                .when(self.upward, |d| d.mb(px(6.)))
                .when(!self.upward, |d| d.mt(px(6.)))
                .on_mouse_down_out(move |_, w, cx| {
                    if let Some(f) = &close {
                        f(false, w, cx)
                    }
                })
                .children(self.items.into_iter().enumerate().flat_map(|(i, item)| {
                    let divider = item.divider.then(|| div().my(px(3.)).mx(px(4.)).h(px(1.)).bg(t.line_strong).into_any_element());
                    if item.heading {
                        let caption = div()
                            .px(px(8.))
                            .pt(px(4.))
                            .pb(px(2.))
                            .text_size(text::CAPS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.fg_3)
                            .child(item.label)
                            .into_any_element();
                        return divider.into_iter().chain([caption]).collect::<Vec<_>>();
                    }
                    let sel = self.on_select.clone();
                    let tog = toggle.clone();
                    let checked = self.selected == Some(i);
                    let fg = if item.danger { t.err } else { t.fg };
                    let row = div()
                        .id((id.clone(), i))
                        .transitions(crate::motion::hovers)
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .min_h(px(30.))
                        .py(px(5.))
                        .px(px(8.))
                        .rounded(radius::SEGMENT)
                        .cursor_pointer()
                        .bg(if item.danger { t.err_soft } else { t.surface_3 }.opacity(0.))
                        .hover_style(move |s| s.bg(if item.danger { t.err_soft } else { t.surface_3 }))
                        .when_some(item.icon, |d, ic| d.child(Icon::new(ic).small().color(if item.danger { t.err } else { t.fg_2 })))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(text::BODY)
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg)
                                        .child(item.label),
                                )
                                .when_some(item.detail, |d, det| {
                                    d.child(div().text_size(text::SMALL).text_color(t.fg_3).child(det))
                                }),
                        )
                        .when(checked, |d| d.child(Icon::new(IconName::Check).small().color(t.fg)))
                        .on_click(move |_, w, cx| {
                            cx.stop_propagation();
                            if let Some(f) = &sel {
                                f(i, w, cx)
                            }
                            if let Some(f) = &tog {
                                f(false, w, cx)
                            }
                        })
                        .into_any_element();
                    divider.into_iter().chain([row]).collect::<Vec<_>>()
                }));
            deferred(
                anchored()
                    .anchor(if self.upward { Anchor::BottomLeft } else { Anchor::TopLeft })
                    .snap_to_window_with_margin(px(8.))
                    .child(crate::motion::pop(list, SharedString::from(format!("{id}-pop")), upward)),
            )
            .into_any_element()
        });
        div()
            .id(self.id)
            .relative()
            .flex_none()
            .child(trigger)
            .when_some(menu, |d, m| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .when(self.upward, |d| d.top_0())
                        .when(!self.upward, |d| d.bottom_0())
                        .child(m),
                )
            })
    }
}
