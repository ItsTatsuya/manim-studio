use studio_ui::motion;
use crate::app::{Workspace, click, display, faint, stage_image, still_image, strong, style_image, sub};
use crate::model::{HOUSE_STYLES, Provider};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder, px, relative,
};
use studio_ui::components::{Button, Chip, ProgressBar, Tone, provider_mark};
use studio_ui::theme::{self, ActiveTheme, radius};
use studio_ui::{Icon, IconName};

pub const STEPS: usize = 5;
/// The natural-voice download step.
const VOICE_STEP: usize = 3;

impl Workspace {
    /// Go to onboarding step `step`; arriving at the voice step starts its download.
    pub fn enter_onboarding_step(&mut self, step: usize, cx: &mut Context<Self>) {
        self.onboarding_step = step.min(STEPS - 1);
        if self.onboarding_step == VOICE_STEP && self.natural == crate::engine::NaturalVoice::Missing {
            self.download_natural_voice(cx);
        }
        cx.notify();
    }

    pub fn render_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let step = self.onboarding_step;
        let body: AnyElement = match step {
            0 => self.onb_welcome(cx),
            1 => self.onb_connect(cx),
            2 => self.onb_style(cx),
            3 => self.onb_voice(cx),
            _ => self.onb_ready(cx),
        };
        let art = match step {
            2 => HOUSE_STYLES[self.prefs.house_style.min(HOUSE_STYLES.len() - 1)].art,
            4 => studio_ui::assets::Art::Quiz,
            _ => studio_ui::assets::Art::Circle,
        };
        let voice_ready = self.natural == crate::engine::NaturalVoice::Ready;
        let can_continue = match step {
            1 => self.providers.iter().any(|p| p.signed_in),
            // Narration needs the natural voice: no way past this step until it's on disk.
            VOICE_STEP => voice_ready,
            _ => true,
        };

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
            // The step scrolls on short windows; Back / Continue stay pinned below it.
            .child(
                div()
                    .id(("onb-body", step))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(motion::enter(div().pb(px(4.)).child(body), ("onb-step", step), motion::BASE, 8.)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .max_w(px(520.))
                    .when(step > 0, |d| {
                        d.child(Button::new("onb-back", "Back").ghost().on_click(click(cx, |this, _, cx| {
                            this.enter_onboarding_step(this.onboarding_step.saturating_sub(1), cx)
                        })))
                    })
                    .child(div().flex_1())
                    // The last step's choices sit where Continue was on every other step.
                    .when(step == STEPS - 1, |d| {
                        d.when(self.sample_video().is_some(), |d| {
                            d.child(Button::new("onb-sample", "Open the sample lesson").ghost().on_click(click(cx, |this, _, cx| {
                                this.finish_onboarding(cx);
                                if let Some((p, v)) = this.sample_video() {
                                    this.open_video(p, v, cx);
                                }
                            })))
                        })
                        .child(
                            Button::new("onb-own", "Start my own")
                                .primary()
                                .large()
                                .icon_right(IconName::ChevronRight)
                                .on_click(click(cx, |this, _, cx| this.finish_onboarding(cx))),
                        )
                    })
                    .when(step < STEPS - 1, |d| {
                        // Skipping still has to stop at the voice download.
                        d.when(step != VOICE_STEP, |d| {
                            d.child(Button::new("onb-skip", "Skip setup").ghost().on_click(click(cx, |this, _, cx| {
                                if this.natural == crate::engine::NaturalVoice::Ready {
                                    this.finish_onboarding(cx);
                                } else {
                                    this.enter_onboarding_step(VOICE_STEP, cx);
                                }
                            })))
                        })
                        .child(
                            Button::new("onb-next", if step == VOICE_STEP && !voice_ready { "Waiting for the voice…" } else { "Continue" })
                                .primary()
                                .large()
                                .disabled(!can_continue)
                                .icon_right(IconName::ChevronRight)
                                .on_click(click(cx, |this, _, cx| this.enter_onboarding_step(this.onboarding_step + 1, cx))),
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
                div()
                    .w(relative(0.78))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(motion::enter(
                        div()
                            .w_full()
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
                    ))
                    .children(self.onb_caption(cx)),
            );

        div()
            .size_full()
            .flex()
            .gap(px(10.))
            .child(left)
            .when(!theme::is_compact(window) || window.viewport_size().width > px(1000.), |d| d.child(right))
            .into_any_element()
    }

    /// Under the preview: a hairline playhead and which scene of the sample is playing.
    fn onb_caption(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let t = cx.theme();
        let video = self.video();
        let scene = video.scenes.get(self.scene)?;
        let total = video.length().max(0.01);
        let progress = (self.playhead / total).clamp(0., 1.);
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .h(px(2.))
                        .w_full()
                        .rounded_full()
                        .bg(t.surface_3)
                        .child(div().h_full().w(relative(progress)).rounded_full().bg(t.fg_3)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(t.fg_3)
                        .child(div().flex_1().min_w_0().truncate().child(format!("Sample lesson · {}", scene.title)))
                        .child(div().flex_none().font_family(studio_ui::theme::FONT_MONO).child(format!("{} / {}", self.scene + 1, video.scenes.len()))),
                )
                .into_any_element(),
        )
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
                    .items_start()
                    .gap(px(6.))
                    .text_color(t.fg_3)
                    .text_size(px(12.))
                    .max_w(px(500.))
                    .child(div().flex_none().pt(px(1.)).child(Icon::new(IconName::Shield).small().color(t.fg_3)))
                    .child(div().flex_1().min_w_0().child(if self.providers.iter().any(|p| p.signed_in) {
                        format!(
                            "You can connect both. Studio uses {} first and switches when a plan runs low.",
                            if self.default_provider == Provider::Claude { "Claude" } else { "ChatGPT" }
                        )
                    } else {
                        "Sign in to one to continue. Sign-in opens a terminal and your browser; Studio notices when you're done.".to_string()
                    })),
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
        // Three per row: six styles fit in two rows.
        const PER_ROW: usize = 3;
        let cards: Vec<_> = cards.collect();
        let mut rows = div().flex().flex_col().gap(px(10.)).max_w(px(560.));
        let mut it = cards.into_iter().peekable();
        while it.peek().is_some() {
            let mut row = div().flex().gap(px(10.));
            for _ in 0..PER_ROW {
                let cell = div().flex_1().min_w_0().flex().flex_col();
                row = row.child(match it.next() {
                    Some(card) => cell.child(card),
                    None => cell,
                });
            }
            rows = rows.child(row);
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

    /// The natural voice (Kokoro) is the only narrator: download it here (it starts on
    /// arrival) before onboarding can finish.
    fn onb_voice(&self, cx: &mut Context<Self>) -> AnyElement {
        use crate::engine::NaturalVoice;
        let t = cx.theme().clone();
        let total_mb = studio_render::kokoro::total_size(self.voice_quality()) as f32 / 1_000_000.;
        let (status, action): (String, Option<AnyElement>) = match &self.natural {
            NaturalVoice::Ready => (
                format!("Installed · {}", self.voice_label()),
                Some(
                    Button::new("onb-voice-try", "Listen")
                        .with_icon(IconName::Play)
                        .on_click(click(cx, |this, _, cx| this.preview_voice(cx)))
                        .into_any_element(),
                ),
            ),
            NaturalVoice::Downloading(f) => (format!("Downloading · {:.0} of {:.0} MB", f * total_mb, total_mb), None),
            NaturalVoice::Missing => (
                format!("One-time download · {total_mb:.0} MB"),
                Some(
                    Button::new("onb-voice-download", "Download")
                        .primary()
                        .with_icon(IconName::Download)
                        .on_click(click(cx, |this, _, cx| this.download_natural_voice(cx)))
                        .into_any_element(),
                ),
            ),
            NaturalVoice::Failed(e) => (
                format!("Download failed: {e}"),
                Some(
                    Button::new("onb-voice-download", "Try again")
                        .primary()
                        .with_icon(IconName::Refresh)
                        .on_click(click(cx, |this, _, cx| this.download_natural_voice(cx)))
                        .into_any_element(),
                ),
            ),
        };
        let (fraction, done) = match &self.natural {
            NaturalVoice::Downloading(f) => (Some(*f), false),
            NaturalVoice::Ready => (None, true),
            _ => (None, false),
        };
        let note = match &self.natural {
            NaturalVoice::Ready => "Ready. You can pick another voice and speed later in Settings → Voice & captions.",
            NaturalVoice::Failed(_) => "Check your internet connection and try again. Studio needs the voice to narrate videos.",
            _ => "Studio needs the voice to narrate videos, so setup continues once it's downloaded. It only happens once.",
        };
        let card = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(14.))
            .max_w(px(500.))
            .rounded(radius::PANEL)
            .bg(t.surface)
            .border_1()
            .border_color(if done { t.fg_3 } else { t.line })
            .shadow(t.edge())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(
                        div()
                            .size(px(40.))
                            .flex_none()
                            .rounded(radius::CONTROL)
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.line)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(if done { IconName::Check } else { IconName::Volume }).color(if done { t.ok } else { t.fg_2 })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(strong("Natural voice").font_weight(gpui::FontWeight::SEMIBOLD))
                            .child(faint(status, cx)),
                    )
                    .when_some(fraction, |d, f| {
                        d.child(div().flex_none().font_family(studio_ui::theme::FONT_MONO).text_size(px(13.)).child(format!("{:.0}%", f * 100.)))
                    })
                    .children(action),
            )
            .when_some(fraction, |d, f| d.child(ProgressBar::new(f).tone(Tone::Neutral).height(6.)));
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(self.onb_heading(
                "Give your lessons a voice.",
                "Narration uses Kokoro, a natural voice that runs on this computer. Nothing you write is sent anywhere.",
                cx,
            ))
            .child(card)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .max_w(px(500.))
                    .text_color(t.fg_3)
                    .text_size(px(12.))
                    .child(div().pt(px(1.)).flex_none().child(Icon::new(IconName::Info).small().color(t.fg_3)))
                    .child(div().flex_1().min_w_0().child(note)),
            )
            .into_any_element()
    }

    fn onb_ready(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let p = self.providers.iter().find(|p| p.provider == self.default_provider && p.signed_in)
            .or_else(|| self.providers.iter().find(|p| p.signed_in));
        let row = |k: &'static str, v: AnyElement| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .min_h(px(36.))
                .border_b_1()
                .border_color(t.line)
                .child(div().text_color(t.fg_2).child(k))
                .child(v)
        };
        let assistant: AnyElement = match p {
            Some(p) => strong(format!("{} · {}", p.provider.name(), p.plan_label())).into_any_element(),
            None => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(faint("Not connected", cx))
                .child(Button::new("onb-connect-again", "Connect").small().on_click(click(cx, |this, _, cx| this.enter_onboarding_step(1, cx))))
                .into_any_element(),
        };
        let (heading, body) = if p.is_some() {
            ("You're set.", "Start your first lesson, or open the sample to see the whole flow first.")
        } else {
            ("Almost there.", "Connect Claude or ChatGPT to write your own lessons. You can open the sample lesson either way.")
        };
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(self.onb_heading(heading, body, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .max_w(px(460.))
                    .child(row("Assistant", assistant))
                    .child(row("House style", strong(HOUSE_STYLES[self.prefs.house_style.min(HOUSE_STYLES.len() - 1)].name).into_any_element()))
                    .child(row("Voice", strong(self.voice_label()).into_any_element()))
                    .child(row(
                        "Rendering",
                        strong(self.engine.backend_info.clone().unwrap_or_else(|| "Checked on first render".into())).into_any_element(),
                    )),
            )
            .into_any_element()
    }
}

