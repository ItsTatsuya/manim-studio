//! Title-bar plan usage: a slim two-bar meter, with a hover card that spells out
//! both windows and when they reset.

use crate::app::{Screen, SettingsPage, Workspace, click, num};
use crate::model::{Provider, ProviderStatus};
use gpui::{
    ColorExt,
    App, AppContext, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px, relative,
};
use studio_ui::components::provider_mark;
use studio_ui::theme::{ActiveTheme, radius, space, text};
use studio_ui::{Icon, motion};

/// Bar colour for a used fraction: calm until the plan is nearly spent.
fn tone(used: f32, cx: &App) -> Hsla {
    let t = cx.theme();
    if used >= 0.9 {
        t.err
    } else if used >= 0.75 {
        t.signal
    } else {
        t.fg_2
    }
}

fn pct(u: f32) -> String {
    format!("{}%", (u * 100.).round() as u32)
}

/// One thin bar whose fill glides to new values.
fn bar(id: &'static str, used: Option<f32>, height: f32, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let u = used.unwrap_or(0.).clamp(0., 1.);
    div().w_full().h(px(height)).rounded_full().bg(t.surface_3).overflow_hidden().child(
        div()
            .id(id)
            .h_full()
            .w(relative(u))
            .rounded_full()
            .bg(tone(u, cx))
            .transitions(|tr| tr.w(motion::SLOW).bg(motion::SLOW)),
    )
}

impl Workspace {
    pub fn usage_meter(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let provider = self.default_provider;
        let status = self.providers.iter().find(|p| p.provider == provider).cloned();
        let window = status.as_ref().and_then(|p| p.window_used);
        let week = status.as_ref().and_then(|p| p.week_used);
        let card_status = status.clone();
        div()
            .id("usage-meter")
            .transitions(motion::hovers)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(space::CONTROL)
            .px(px(9.))
            .rounded(radius::CONTROL)
            .text_color(t.fg_2)
            .cursor_pointer()
            .bg(t.surface_2.opacity(0.))
            .hover_style(|s| s.bg(t.surface_2).text_color(t.fg))
            .on_click(click(cx, |this, _, cx| {
                this.settings_page = SettingsPage::Assistants;
                this.go(Screen::Settings, cx);
            }))
            .child(Icon::new(provider.icon()).small())
            .when(!compact, |d| {
                // 5-hour window on top, the week underneath: one glance shows both.
                d.child(
                    div()
                        .w(px(38.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(bar("usage-bar-window", window, 3., cx))
                        .child(bar("usage-bar-week", week, 3., cx)),
                )
            })
            .child(
                num(window.map(pct).unwrap_or_else(|| "—".into()))
                    .text_size(text::SMALL)
                    .font_weight(FontWeight::MEDIUM),
            )
            .tooltip(move |_, cx| {
                let status = card_status.clone();
                cx.new(|_| UsageCard { provider, status }).into()
            })
    }
}

/// The hover card: both windows with their reset times.
struct UsageCard {
    provider: Provider,
    status: Option<ProviderStatus>,
}

impl UsageCard {
    fn window_row(
        &self,
        id: &'static str,
        label: &'static str,
        used: Option<f32>,
        resets: Option<String>,
        cx: &App,
    ) -> impl IntoElement {
        let t = cx.theme();
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .child(div().flex_1().text_color(t.fg).font_weight(FontWeight::MEDIUM).child(label))
                    .child(
                        num(used.map(pct).unwrap_or_else(|| "—".into()))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(used.map_or(t.fg_3, |u| if u >= 0.75 { tone(u, cx) } else { t.fg })),
                    ),
            )
            .child(bar(id, used, 5., cx))
            .child(
                div().text_size(text::SMALL).text_color(t.fg_3).child(match (resets, used) {
                    (Some(r), _) => format!("Resets in {r}"),
                    (None, Some(_)) => "Reset time not reported yet".to_string(),
                    (None, None) => "Reported after the next assistant turn".to_string(),
                }),
            )
    }
}

impl Render for UsageCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let s = self.status.clone();
        let plan = s.as_ref().and_then(|s| s.plan.clone());
        let window = s.as_ref().and_then(|s| s.window_used);
        let week = s.as_ref().and_then(|s| s.week_used);
        let window_resets = s.as_ref().and_then(|s| s.resets_in());
        let week_resets = s.as_ref().and_then(|s| s.week_resets_in());
        let card = div()
            .w(px(264.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(14.))
            .rounded(radius::PANEL)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow(t.pop_shadow())
            .text_size(text::BODY)
            .text_color(t.fg_2)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(provider_mark(self.provider.icon(), 20., cx))
                    .child(div().flex_1().text_color(t.fg).font_weight(FontWeight::SEMIBOLD).child(self.provider.name()))
                    .when_some(plan, |d, p| {
                        d.child(
                            div()
                                .px(px(7.))
                                .py(px(1.))
                                .rounded(radius::CHIP)
                                .bg(t.surface_3)
                                .text_size(text::SMALL)
                                .text_color(t.fg_2)
                                .child(format!("{p} plan")),
                        )
                    }),
            )
            .child(self.window_row("usage-card-window", "5-hour window", window, window_resets, cx))
            .child(self.window_row("usage-card-week", "This week", week, week_resets, cx))
            .child(
                div()
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(t.line)
                    .text_size(text::SMALL)
                    .text_color(t.fg_3)
                    .child("Updated after each turn · Click for usage settings"),
            );
        div().pt(px(8.)).child(motion::pop(card, "usage-card", false))
    }
}
