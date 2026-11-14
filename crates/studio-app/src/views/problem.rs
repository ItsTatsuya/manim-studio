//! An assistant failure, explained: what happened, what it means, and buttons that fix
//! it. Shown in the chat and on the storyboard when a plan never arrived.

use crate::app::{Screen, SettingsPage, Workspace, click, faint, strong, sub};
use crate::model::{Msg, Provider};
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder, px};
use studio_agent::errors::Fix;
use studio_ui::components::Button;
use studio_ui::theme::{ActiveTheme, FONT_MONO, radius, text};
use studio_ui::{Icon, IconName};

impl Workspace {
    /// `actions`: show the fix buttons (only on the latest failure; older ones are history).
    pub fn problem_card(&self, key: usize, provider: Provider, raw: &str, actions: bool, cx: &Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let e = Msg::explain(provider, raw);
        let other = self.other_provider(provider);
        let mut buttons: Vec<AnyElement> = Vec::new();
        if actions {
            for fix in &e.fixes {
                let first = buttons.is_empty();
                let id = ("problem-fix", key * 16 + buttons.len());
                let button = match fix {
                    Fix::SignIn => Some(
                        Button::new(id, format!("Sign in to {}", provider.name()))
                            .with_icon(IconName::Key)
                            .on_click(click(cx, move |this, _, cx| this.sign_in_provider(provider, cx))),
                    ),
                    Fix::Install => Some(
                        Button::new(id, format!("Install {}", provider.cli()))
                            .with_icon(IconName::Download)
                            .on_click(click(cx, move |this, _, cx| this.install_provider(provider, cx))),
                    ),
                    Fix::Update => Some(
                        Button::new(id, format!("Update {}", provider.cli()))
                            .with_icon(IconName::Download)
                            .on_click(click(cx, move |this, _, cx| this.install_provider(provider, cx))),
                    ),
                    Fix::Open { label, url } => {
                        let url = *url;
                        Some(Button::new(id, *label).with_icon(IconName::External).on_click(click(cx, move |_, _, _| crate::crash::open_url(url))))
                    }
                    Fix::Retry if self.can_retry() && !self.busy(self.current) => Some(
                        Button::new(id, "Try again")
                            .with_icon(IconName::Refresh)
                            .on_click(click(cx, |this, _, cx| this.retry_failed(cx))),
                    ),
                    Fix::SwitchProvider => other.map(|o| {
                        Button::new(id, format!("Use {} instead", o.name())).on_click(click(cx, move |this, _, cx| {
                            this.set_default_provider(o);
                            this.retry_failed(cx);
                        }))
                    }),
                    Fix::Settings => Some(Button::new(id, "Assistant settings").on_click(click(cx, |this, _, cx| {
                        this.settings_page = SettingsPage::Assistants;
                        this.go(Screen::Settings, cx);
                    }))),
                    Fix::Retry => None,
                };
                if let Some(b) = button {
                    // The first fix is the one to try; the rest stay quiet.
                    buttons.push(if first { b.primary().into_any_element() } else { b.ghost().into_any_element() });
                }
                if buttons.len() == 3 {
                    break;
                }
            }
        }
        // The provider's own words, trimmed: useful when reporting a problem.
        let details: String = raw.trim().chars().take(240).collect();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(14.))
            .rounded(radius::PANEL)
            .bg(t.surface_2)
            .border_1()
            .border_color(t.line_strong)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(div().pt(px(2.)).flex_none().child(Icon::new(IconName::Info).color(t.err)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(strong(e.title))
                            .child(sub(e.body, cx)),
                    ),
            )
            .when(!buttons.is_empty(), |d| d.child(div().flex().flex_wrap().gap(px(8.)).pl(px(26.)).children(buttons)))
            .child(
                div()
                    .pl(px(26.))
                    .child(faint(format!("{}: {details}", provider.cli()), cx).font_family(FONT_MONO).text_size(text::CAPS)),
            )
            .into_any_element()
    }
}
