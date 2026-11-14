use studio_ui::motion;
use crate::app::{Workspace, click, display, faint, stage_image, still_image, strong, style_image, sub};
use crate::model::{HOUSE_STYLES, Provider};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder, px, relative,
};
use studio_ui::components::{Button, Chip, Dot, Tone, floating, provider_mark};
use studio_ui::theme::{self, ActiveTheme, radius};
use studio_ui::{Icon, IconName};

const STEPS: usize = 4;

impl Workspace {
    pub fn render_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let step = self.onboarding_step;
        let body: AnyElement = match step {
            0 => self.onb_welcome(cx),
            1 => self.onb_connect(cx),
            2 => self.onb_style(cx),
            _ => self.onb_ready(cx),
        };
        let art = match step {
            2 => HOUSE_STYLES[self.prefs.house_style.min(HOUSE_STYLES.len() - 1)].art,
            3 => studio_ui::assets::Art::Quiz,
            _ => studio_ui::assets::Art::Circle,
        };
        let can_continue = step != 1 || self.providers.iter().any(|p| p.signed_in);

        let left = div()
            .w(relative(0.5))
            .flex_none()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(24.))
            .px(px(64.))
            .py(px(56.))
            .child(
                div().flex().gap(px(6.)).children((0..STEPS).map(|i| {
                    div()
                        .h(px(3.))
                        .w(px(28.))
                        .rounded_full()
                        .bg(if i <= step { t.fg } else { t.surface_3 })
                })),
            )
            .child(motion::enter(div().child(body), ("onb-step", step), motion::BASE, 8.))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .max_w(px(520.))
                    .when(step > 0, |d| {
                        d.child(Button::new("onb-back", "Back").ghost().on_click(click(cx, |this, _, cx| {
                            this.onboarding_step -= 1;
                            cx.notify();
                        })))
                    })
                    .child(div().flex_1())
                    .when(step < STEPS - 1, |d| {
                        d.child(Button::new("onb-skip", "Skip setup").ghost().on_click(click(cx, |this, _, cx| this.finish_onboarding(cx))))
                            .child(
                                Button::new("onb-next", "Continue")
                                    .primary()
                                    .large()
                                    .disabled(!can_continue)
                                    .icon_right(IconName::ChevronRight)
                                    .on_click(click(cx, |this, _, cx| {
                                        this.onboarding_step += 1;
                                        cx.notify();
                                    })),
                            )
                    }),
            );

        let right = div()
            .flex_1()
            .flex_basis(px(0.))
            .min_w_0()
            .relative()
            .rounded(radius::PANEL)
            .bg(t.stage)
            .border_1()
            .border_color(t.line)
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .child(
                motion::enter(
                    div()
                        .w(relative(0.78))
                        .aspect_ratio(16. / 9.)
                        .rounded(radius::INNER)
                        .overflow_hidden()
                        .border_1()
                        .border_color(t.line)
                        .child(match &self.stage.frame {
                            Some(img) => still_image(img.clone()).into_any_element(),
                            None => stage_image(art).into_any_element(),
                        }),
                    ("onb-art", self.stage.frame.is_some() as usize),
                    motion::SLOW,
                    0.,
                ),
            )
            .child(
                floating(cx)
                    .absolute()
                    .left(relative(0.07))
                    .top(relative(0.12))
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(12.))
                    .child(Dot::new(Tone::Signal))
                    .child("Writing storyboard · 5 scenes"),
            )
            .child(
                floating(cx)
                    .absolute()
                    .right(relative(0.07))
                    .bottom(relative(0.14))
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(12.))
                    .child(Icon::new(IconName::Check).small().color(t.ok))
                    .child("Rendered 1080p in 3.4 s"),
            );

        div()
            .size_full()
            .flex()
            .gap(px(10.))
            .child(left)
            .when(!theme::is_compact(window) || window.viewport_size().width > px(1000.), |d| d.child(right))
            .into_any_element()
    }

    fn onb_heading(&self, h: &'static str, body: &'static str, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .max_w(px(440.))
            .child(display(h))
            .child(sub(body, cx).text_size(px(14.)).line_height(px(21.)))
    }

    fn onb_welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let point = |icon: IconName, head: &'static str, body: &'static str| {
            div()
                .flex()
                .gap(px(12.))
                .child(
                    div()
                        .size(px(32.))
                        .flex_none()
                        .rounded(radius::CONTROL)
                        .bg(t.surface_2)
                        .border_1()
                        .border_color(t.line)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(icon).color(t.fg_2)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(strong(head))
                        .child(div().text_color(t.fg_3).text_size(px(12.)).child(body)),
                )
        };
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(self.onb_heading(
                "Lessons students remember,\nin minutes.",
                "Describe a topic, drop in your notes, or paste a script. Studio plans the video, animates it, narrates it, and renders it on this computer.",
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .max_w(px(460.))
                    .child(point(IconName::Board, "You approve a storyboard first", "Nothing is built until the plan looks right. That keeps usage low."))
                    .child(point(IconName::Key, "Uses the plan you already have", "Claude or ChatGPT. Studio never stores your password."))
                    .child(point(IconName::Film, "Renders locally, fast", "1080p in seconds, with captions and a natural voice.")),
            )
            .into_any_element()
    }

    fn onb_connect(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let cards = self.providers.iter().enumerate().map(|(i, p)| {
            let selected = p.provider == self.default_provider && p.signed_in;
            let caption = match (&p.version, p.checking, p.installed) {
                (_, true, false) => "Looking for it…".to_string(),
                (Some(v), _, _) => match &p.plan {
                    Some(plan) if p.signed_in => format!("{} {v} · {plan}", p.provider.cli()),
                    _ => format!("{} {v}", p.provider.cli()),
                },
                (None, _, true) => p.provider.cli().to_string(),
                (None, _, false) => format!("{} isn't installed", p.provider.cli()),
            };
            let provider_kind = p.provider;
            let action: AnyElement = if p.signed_in {
                Chip::new("Signed in").tone(Tone::Ok).icon(IconName::Check).into_any_element()
            } else if p.installed {
                Button::new(("connect", i), "Sign in")
                    .on_click(click(cx, move |this, _, cx| this.sign_in_provider(provider_kind, cx)))
                    .into_any_element()
            } else {
                Button::new(("connect", i), "Install")
                    .on_click(click(cx, move |this, _, cx| this.install_provider(provider_kind, cx)))
                    .into_any_element()
            };
            let provider = p.provider;
            div()
                .id(("provider-card", i))
                .flex()
                .items_center()
                .gap(px(14.))
                .p(px(14.))
                .rounded(radius::PANEL)
                .bg(t.surface)
                .border_1()
                .border_color(if selected { t.fg_3 } else { t.line })
                .shadow(t.edge())
                .when(p.signed_in, |d| {
                    d.cursor_pointer().on_click(click(cx, move |this, _, cx| {
                        this.set_default_provider(provider);
                        cx.notify();
                    }))
                })
                .child(provider_mark(p.provider.icon(), 40., cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(strong(p.provider.name()).font_weight(gpui::FontWeight::SEMIBOLD))
                        .child(faint(caption, cx)),
                )
                .child(action)
        });
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(self.onb_heading(
                "Connect the assistant\nyou already pay for.",
                "Studio uses your existing ChatGPT or Claude plan to write lessons. Nothing is billed separately, and your sign-in stays with the provider.",
                cx,
            ))
            .child(div().flex().flex_col().gap(px(10.)).max_w(px(500.)).children(cards))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_color(t.fg_3)
                    .text_size(px(12.))
                    .child(Icon::new(IconName::Shield).small().color(t.fg_3))
                    .child(format!(
                        "You can connect both. Studio uses {} first and switches when a plan runs low.",
                        if self.default_provider == Provider::Claude { "Claude" } else { "ChatGPT" }
                    )),
            )
            .into_any_element()
    }

    fn onb_style(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let cards = HOUSE_STYLES.iter().enumerate().map(|(i, s)| {
            let on = self.prefs.house_style == i;
            div()
                .id(("style", i))
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(6.))
                .pb(px(10.))
                .rounded(radius::PANEL)
                .bg(if on { t.surface_3 } else { t.surface })
                .border_1()
                .border_color(if on { t.fg_3 } else { t.line })
                .cursor_pointer()
                .hover_style(|s| s.border_color(t.line_strong))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| this.hover_style_card(i, *hovered, cx)))
                .on_click(click(cx, move |this, _, cx| {
                    this.prefs.house_style = i;
                    this.save_prefs();
                    this.restyle(cx);
                }))
                .child(
                    div()
                        .w_full()
                        .aspect_ratio(16. / 9.)
                        .rounded(radius::THUMB)
                        .overflow_hidden()
                        .child(style_image(self, i, s.art)),
                )
                .child(
                    div()
                        .px(px(4.))
                        .flex()
                        .flex_col()
                        .child(strong(s.name))
                        .child(faint(s.blurb, cx)),
                )
        });
        let cards: Vec<_> = cards.collect();
        let mut rows = div().flex().flex_col().gap(px(10.)).max_w(px(520.));
        let mut it = cards.into_iter();
        while let Some(a) = it.next() {
            let b = it.next();
            rows = rows.child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(div().flex_1().min_w_0().flex().flex_col().child(a))
                    .child(div().flex_1().min_w_0().flex().flex_col().when_some(b, |d, b| d.child(b))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(self.onb_heading(
                "Pick a house style.",
                "Fonts, colours and layout for your videos. You can change it for any video later.",
                cx,
            ))
            .child(rows)
            .into_any_element()
    }

    fn onb_ready(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let p = self.providers.iter().find(|p| p.provider == self.default_provider);
        let row = |k: &'static str, v: String| {
            div()
                .flex()
                .justify_between()
                .py(px(8.))
                .border_b_1()
                .border_color(t.line)
                .child(div().text_color(t.fg_2).child(k))
                .child(strong(v))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(self.onb_heading(
                "You're set.",
                "Start with a sample lesson to see the whole flow, or go straight to your own.",
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .max_w(px(460.))
                    .child(row(
                        "Assistant",
                        p.filter(|p| p.signed_in).map_or("Not connected".into(), |p| format!("{} · {}", p.provider.name(), p.plan_label())),
                    ))
                    .child(row("House style", HOUSE_STYLES[self.prefs.house_style.min(HOUSE_STYLES.len() - 1)].name.into()))
                    .child(row("Voice", self.voice_label()))
                    .child(row("Rendering", self.engine.backend_info.clone().unwrap_or_else(|| "Checked on first render".into()))),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("onb-sample", "Open the sample lesson")
                            .primary()
                            .large()
                            .on_click(click(cx, |this, _, cx| {
                                this.finish_onboarding(cx);
                                if let Some((p, v)) = this.sample_video() {
                                    this.open_video(p, v, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("onb-own", "Start my own")
                            .large()
                            .on_click(click(cx, |this, _, cx| this.finish_onboarding(cx))),
                    ),
            )
            .into_any_element()
    }
}

