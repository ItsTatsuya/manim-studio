use crate::app::{Workspace, click, display, faint, num, strong, sub, title};
use crate::model::format_short;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder, px,
};
use studio_ui::components::{Button, Chip, Tone, caps, panel};
use studio_ui::theme::{self, ActiveTheme, FONT_MONO, radius, space, text};
use studio_ui::{Icon, IconName};

impl Workspace {
    pub fn render_storyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let compact = theme::is_compact(window);
        let video = self.video().clone();
        let busy = video.busy.clone();
        let planning = matches!(busy, Some(crate::model::Busy::Planning) | Some(crate::model::Busy::Revising));
        let beats: Vec<_> = video.scenes.iter().enumerate().map(|(i, s)| {
            let on = i == self.scene;
            div()
                .id(("beat", i))
                .flex()
                .items_start()
                .gap(px(16.))
                .p(px(12.))
                .rounded(radius::PANEL)
                .bg(t.surface_2)
                .border_1()
                .border_color(if on { t.fg_3 } else { t.line })
                .cursor_pointer()
                .when(!on, |d| d.hover_style(|s| s.border_color(t.line_strong)))
                .on_click(click(cx, move |this, _, cx| this.seek_scene(i, cx)))
                .child(
                    div()
                        .size(px(26.))
                        .flex_none()
                        .rounded(radius::CHIP)
                        .bg(t.surface_3)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(text::SMALL)
                        .child(strong((i + 1).to_string())),
                )
                .when(!compact, |d| {
                    d.child(
                        div()
                            .w(px(170.))
                            .flex_none()
                            .aspect_ratio(16. / 9.)
                            .rounded(radius::CHIP)
                            .overflow_hidden()
                            .child(crate::app::scene_image(s, cx)),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(title(s.title.clone()).text_size(px(14.)))
                        .child(sub(format!("“{}”", s.say), cx))
                        .child(faint(s.visual.clone(), cx))
                        .when_some(s.note.clone(), |d, n| d.child(div().flex().mt(px(4.)).child(Chip::new(n).tone(Tone::Signal)))),
                )
                .child(num(format_short(s.seconds)).text_color(t.fg_3).flex_none())
        }).collect();
        // While the plan is being written: what was asked, and skeleton beats.
        let skeleton = (planning && beats.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .when_some(video.request.clone(), |d, r| d.child(faint(format!("“{r}”"), cx)))
                .children((0..4usize).map(|i| {
                    div()
                        .id(("beat-skeleton", i))
                        .h(px(118.))
                        .rounded(radius::PANEL)
                        .bg(t.surface_2)
                        .border_1()
                        .border_color(t.line)
                }))
        });
        // The plan never arrived (assistant error, plan run out, sign-in…): say why, and
        // offer to try again instead of leaving an empty page.
        let failed = (!planning && beats.is_empty() && video.storyboard.is_none() && video.request.is_some()).then(|| {
            let reason = self.last_error(&video).unwrap_or_else(|| "The assistant stopped before the storyboard was written.".into());
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(16.))
                .rounded(radius::PANEL)
                .bg(t.surface_2)
                .border_1()
                .border_color(t.line_strong)
                .when_some(video.request.clone(), |d, r| d.child(faint(format!("“{r}”"), cx)))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(10.))
                        .child(div().pt(px(2.)).child(Icon::new(IconName::Info).color(t.err)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(strong("The storyboard couldn't be written"))
                                .child(sub(reason, cx)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            Button::new("plan-retry", "Try again")
                                .primary()
                                .with_icon(IconName::Refresh)
                                .on_click(click(cx, |this, _, cx| this.retry_plan(cx))),
                        )
                        .child(Button::new("plan-settings", "Assistant settings").ghost().on_click(click(cx, |this, _, cx| {
                            this.settings_page = crate::app::SettingsPage::Assistants;
                            this.go(crate::app::Screen::Settings, cx);
                        }))),
                )
        });
        // Rough: the scene API context plus writing each scene (cached context is cheap).
        let est_tokens = 20_000 + video.scenes.len() as u64 * 8_000;
        let window_pct = self
            .providers
            .iter()
            .find(|p| p.provider == self.default_provider)
            .and_then(|p| p.window_used)
            .map(|w| format!("Your {} plan's 5-hour window is at {}%", self.default_provider.name(), (w * 100.).round() as u32))
            .unwrap_or_else(|| "Usage shows after the first turn.".into());

        let list = panel(cx)
            .flex_1()
            .child(
                div()
                    .id("beats")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(22.))
                    .pt(px(18.))
                    .pb(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(12.))
                            .mb(px(4.))
                            .child(display(video.title.clone()).text_size(px(22.)).line_height(px(28.)))
                            .child(faint(
                                [
                                    Some(format!("{} scenes", video.scenes.len())),
                                    Some(format!("about {}", format_short(video.seconds()))),
                                    (!video.grade.trim().is_empty()).then(|| video.grade.clone()),
                                ]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" · "),
                                cx,
                            )
                            .pb(px(4.))),
                    )
                    .children(beats)
                    .children(skeleton)
                    .children(failed),
            )
            .child(
                div()
                    .mx(px(22.))
                    .mb(px(18.))
                    .mt(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(10.))
                    .pl(px(12.))
                    .rounded(radius::PANEL)
                    .bg(t.surface_2)
                    .border_1()
                    .border_color(t.line_strong)
                    .cursor_text()
                    .on_mouse_down(gpui::MouseButton::Left, crate::app::focus_on_click(&self.revise_input))
                    .child(self.revise_input.clone())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(self.tier_menu("revise-model", self.prefs.planning_tier, self.default_provider, true, true, cx, |p, v| p.planning_tier = v))
                            .child(div().flex_1())
                            .child(
                                Button::icon("revise-send", IconName::Up)
                                    .primary()
                                    .small()
                                    .tooltip("Revise the plan")
                                    .disabled(self.revise_input.read(cx).text().trim().is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let text = this.revise_input.read(cx).text().trim().to_string();
                                        if !text.is_empty() {
                                            this.revise_input.update(cx, |i, cx| i.clear(cx));
                                            this.revise_storyboard(text, cx);
                                        }
                                    })),
                            ),
                    ),
            );

        let kv = |k: &'static str, v: String| {
            div()
                .flex()
                .justify_between()
                .py(px(7.))
                .child(div().text_color(t.fg_2).child(k))
                .child(strong(v))
        };
        let kv_el = |k: &'static str, v: AnyElement| {
            div().flex().items_center().justify_between().py(px(3.)).child(div().text_color(t.fg_2).child(k)).child(v)
        };
        let (style_menu, output_menu) = self.video_setting_menus(("sb-style", "sb-output"), false, cx);
        let section = || div().px(px(16.)).py(px(14.)).border_b_1().border_color(t.line).flex().flex_col();

        let side = panel(cx)
            .w(px(340.))
            .flex_none()
            .child(
                section()
                    .gap(px(4.))
                    .child(title("Before you build").text_size(px(14.)))
                    .child(faint("The storyboard is a cheap plan. Code is written only after you approve it.", cx)),
            )
            .child(
                section()
                    .gap(px(6.))
                    .child(caps("Estimated build", cx))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(6.))
                            .child(display(format!("~{}", crate::model::format_tokens(est_tokens))).text_size(px(24.)).line_height(px(28.)))
                            .child(sub("tokens", cx).pb(px(3.))),
                    )
                    .child(faint(window_pct, cx)),
            )
            .child(
                section()
                    .child(kv_el("House style", style_menu.into_any_element()))
                    .child(kv("Voice", self.voice_label()))
                    .child(kv_el("Output", output_menu.into_any_element()))
                    .child(kv("Captions", crate::engine::CAPTIONS[self.prefs.captions.min(2)].to_string()))
                    .child(kv_el("Watermark", self.watermark_menu("sb-watermark", false, cx).into_any_element()))
                    .child(kv("Model", format!("{} for build", self.prefs.build_tier.model(self.default_provider)))),
            )
            .child(
                div()
                    .px(px(16.))
                    .py(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(caps("Sources", cx))
                    .when(video.source.is_none(), |d| d.child(faint("No documents attached.", cx)))
                    // One row per document (`source` is the names joined with ", ").
                    .children(video.source.iter().flat_map(|s| s.split(", ")).filter(|s| !s.is_empty()).map(|s| {
                        let kind = std::path::Path::new(s)
                            .extension()
                            .map(|e| e.to_string_lossy().to_uppercase())
                            .filter(|e| e.len() <= 4)
                            .unwrap_or_else(|| "DOC".into());
                        let s = s.to_string();
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .p(px(8.))
                            .rounded(radius::CARD)
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.line)
                            .child(
                                div()
                                    .w(px(30.))
                                    .h(px(34.))
                                    .rounded(radius::THUMB)
                                    .bg(t.surface_3)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(text::CAPS)
                                    .text_color(t.fg_2)
                                    .font_family(FONT_MONO)
                                    .child(kind),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(strong(s).text_size(text::SMALL))
                                    .child(faint("Read into the plan", cx).text_size(text::CAPS)),
                            )
                    })),
            );

        div()
            .size_full()
            .flex()
            .gap(space::GUTTER)
            .child(list)
            .when(!compact, |d| d.child(side))
            .into_any_element()
    }
}
