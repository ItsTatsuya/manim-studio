use crate::theme::radius;
use crate::theme::{ActiveTheme, text};
use gpui::{
    AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, px,
};

/// A small label shown after hovering a control.
pub struct Tooltip {
    label: SharedString,
}

impl Tooltip {
    /// Use with `.tooltip(Tooltip::text("Label"))` on any interactive element.
    pub fn text(label: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let label = label.into();
        move |_, cx| cx.new(|_| Tooltip { label: label.clone() }).into()
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div().pl(px(8.)).pt(px(10.)).child(crate::motion::pop(
            div()
                .px(px(8.))
                .py(px(4.))
                .rounded(radius::CHIP)
                .bg(t.raised)
                .border_1()
                .border_color(t.line_strong)
                .shadow(t.pop_shadow())
                .text_size(text::SMALL)
                .text_color(t.fg)
                .whitespace_nowrap()
                .child(self.label.clone()),
            "tooltip",
            false,
        ))
    }
}
