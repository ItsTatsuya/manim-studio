use crate::icon::{Icon, IconName};
use crate::theme::{ActiveTheme, num_features, radius, space, text};
use gpui::{
    ColorExt,
    AnyElement, App, ClickEvent, Div, Stateful, ElementId, FontWeight, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

/// A sidebar / list row: leading icon or element, label, trailing meta.
#[derive(IntoElement)]
pub struct ListRow {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    leading: Option<AnyElement>,
    meta: Option<SharedString>,
    indent: bool,
    selected: bool,
    /// Replaces the label (an inline rename field).
    editor: Option<AnyElement>,
    /// Shown at the right edge on hover (row actions), over the meta.
    actions: Option<AnyElement>,
    /// Keep the actions visible (their menu is open).
    actions_shown: bool,
    on_click: Option<crate::components::button::ClickHandler>,
    on_right_click: Option<RightClick>,
    /// Extra behaviour on the row itself (drag source, drop target).
    decorate: Option<Decorate>,
}

type Decorate = Box<dyn FnOnce(Stateful<Div>) -> Stateful<Div>>;
type RightClick = std::rc::Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

impl ListRow {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            leading: None,
            meta: None,
            indent: false,
            selected: false,
            editor: None,
            actions: None,
            actions_shown: false,
            on_click: None,
            on_right_click: None,
            decorate: None,
        }
    }

    pub fn icon(mut self, i: IconName) -> Self {
        self.icon = Some(i);
        self
    }

    pub fn leading(mut self, el: impl IntoElement) -> Self {
        self.leading = Some(el.into_any_element());
        self
    }

    pub fn meta(mut self, m: impl Into<SharedString>) -> Self {
        self.meta = Some(m.into());
        self
    }

    pub fn indent(mut self) -> Self {
        self.indent = true;
        self
    }

    pub fn selected(mut self, s: bool) -> Self {
        self.selected = s;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(f));
        self
    }

    pub fn on_right_click(mut self, f: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_right_click = Some(std::rc::Rc::new(f));
        self
    }

    /// Add behaviour to the row element, e.g. `.on_drag(..)` / `.on_drop(..)`.
    pub fn decorate(mut self, f: impl FnOnce(Stateful<Div>) -> Stateful<Div> + 'static) -> Self {
        self.decorate = Some(Box::new(f));
        self
    }

    pub fn editor(mut self, el: impl IntoElement) -> Self {
        self.editor = Some(el.into_any_element());
        self
    }

    pub fn actions(mut self, el: impl IntoElement, shown: bool) -> Self {
        self.actions = Some(el.into_any_element());
        self.actions_shown = shown;
        self
    }
}

const GROUP: &str = "list-row";

impl RenderOnce for ListRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme().clone();
        let sel = self.selected;
        let has_actions = self.actions.is_some();
        let shown = self.actions_shown;
        let editing = self.editor.is_some();
        div()
            .id(self.id)
            .group(GROUP)
            .relative()
            .transitions(crate::motion::hovers)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(9.))
            .h(space::ROW)
            .pl(if self.indent { px(28.) } else { px(8.) })
            .pr(px(8.))
            .rounded(radius::CONTROL)
            .text_size(text::BODY)
            .whitespace_nowrap()
            .overflow_hidden()
            .when(!editing, |d| d.cursor_pointer())
            .when(sel, |d| d.bg(t.surface_3).text_color(t.fg).font_weight(FontWeight::MEDIUM))
            .when(!sel, |d| d.bg(t.surface_2.opacity(0.)).text_color(t.fg_2).hover_style(|s| s.bg(t.surface_2).text_color(t.fg)))
            .when_some(self.icon, |d, i| d.child(Icon::new(i).color(if sel { t.fg } else { t.fg_2 })))
            .when_some(self.leading, |d, el| d.child(el))
            .map(|d| match self.editor {
                Some(ed) => d.child(div().flex_1().min_w_0().child(ed)),
                None => d.child(div().flex_1().min_w_0().truncate().child(self.label)),
            })
            .when_some(self.meta.filter(|_| !editing), |d, m| {
                d.child(
                    div()
                        .id("meta")
                        .flex_none()
                        .text_size(px(11.))
                        .font_weight(FontWeight::NORMAL)
                        .font_features(num_features())
                        .text_color(t.fg_3)
                        .transitions(crate::motion::fades)
                        // Row actions take the meta's place on hover.
                        .opacity(1.)
                        .when(has_actions && shown, |d| d.opacity(0.))
                        .when(has_actions && !shown, |d| d.group_hover_style(GROUP, |s| s.opacity(0.)))
                        .child(m),
                )
            })
            .when_some(self.actions.filter(|_| !editing), |d, a| {
                d.child(
                    div()
                        .id("actions")
                        .absolute()
                        .right(px(3.))
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .gap(px(1.))
                        .pl(px(4.))
                        .rounded(radius::CONTROL)
                        .bg(if sel { t.surface_3 } else { t.surface_2 })
                        .transitions(crate::motion::fades)
                        .opacity(1.)
                        .when(!shown, |d| d.opacity(0.).group_hover_style(GROUP, |s| s.opacity(1.)))
                        .child(a),
                )
            })
            .when_some(self.on_right_click, |d, f| d.on_mouse_down(MouseButton::Right, move |e, w, cx| f(e, w, cx)))
            .when_some(self.on_click.filter(|_| !editing), |d, f| d.on_click(move |e, w, cx| f(e, w, cx)))
            .when_some(self.decorate.filter(|_| !editing), |d, f| f(d))
    }
}
