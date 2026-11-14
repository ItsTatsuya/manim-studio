use studio_ui::motion;
use crate::app::{Screen, SettingsPage, Workspace, click, faint, num, strong, sub, title, with_value};
use crate::model::{HOUSE_STYLES, Provider, Reasoning, TURN_BUDGETS, Tier};
use crate::prefs::Prefs;
use gpui::{
    AnyElement, App, Context, Div, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use studio_ui::ThemeMode;
use studio_ui::theme::Palette;
use studio_ui::components::{
    Button, Chip, Dropdown, Kbd, caps, ListRow, MenuItem, ProgressBar, Segment, Segmented, Toggle, Tone, panel,
    provider_mark,
};
use studio_ui::theme::{ActiveTheme, radius, space, text};
use studio_ui::{Icon, IconName};

fn group(cx: &App) -> Div {
    let t = cx.theme();
    div()
        .flex()
        .flex_col()
        .rounded(radius::PANEL)
        .bg(t.surface_2)
        .border_1()
        .border_color(t.line)
}

fn item(head: &'static str, desc: impl Into<gpui::SharedString>, control: impl IntoElement, last: bool, cx: &App) -> Div {
    let t = cx.theme();
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .px(px(14.))
        .py(px(12.))
        .when(!last, |d| d.border_b_1().border_color(t.line))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(strong(head))
                .child(faint(desc, cx)),
        )
        .child(control)
}

fn section(label: &'static str, cx: &App) -> Div {
    div().px(px(2.)).mb(px(-8.)).child(caps(label, cx))
}

fn pair(a: impl IntoElement, b: impl IntoElement) -> Div {
    div().flex().items_center().gap(px(6.)).child(a).child(b)
}

impl Workspace {
    pub fn render_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let page = self.settings_page;
        let nav = div()
            .w(px(220.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.))
            .pt(px(2.))
            .children(SettingsPage::ALL.iter().enumerate().map(|(i, p)| {
                let p = *p;
                ListRow::new(("settings-nav", i), p.label())
                    .icon(p.icon())
                    .selected(p == page)
                    .on_click(click(cx, move |this, _, cx| {
                        this.settings_page = p;
                        this.open_menu = None;
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(
                ListRow::new("settings-back", "Back to Studio")
                    .icon(IconName::ChevronLeft)
                    .on_click(click(cx, |this, _, cx| this.go(Screen::Video, cx))),
            );

        let (heading, blurb, body): (&str, &str, AnyElement) = match page {
            SettingsPage::General => ("General", "Appearance and how videos move through Studio.", self.settings_general(cx)),
            SettingsPage::Assistants => (
                "Assistants & usage",
                "Studio signs in through the official CLIs and never stores your credentials.",
                self.settings_assistants(cx),
            ),
            SettingsPage::Styles => ("House styles", "Fonts, colours and layout for new videos. Each video can pick its own.", self.settings_styles(cx)),
            SettingsPage::Voice => ("Voice & captions", "Narration is generated on this computer. No audio leaves it.", self.settings_voice(cx)),
            SettingsPage::Rendering => ("Rendering", "How the engine renders your videos.", self.settings_rendering(cx)),
            SettingsPage::Shortcuts => (
                "Keyboard shortcuts",
                "Single keys work when you're not typing in a text field.",
                self.settings_shortcuts(cx),
            ),
            SettingsPage::Updates => ("Updates", "Studio updates itself in the background and applies updates on restart.", self.settings_updates(cx)),
            SettingsPage::About => ("About", "Manim Studio, built in Rust on GPUI and manim-rust.", self.settings_about(cx)),
        };

        div()
            .size_full()
            .flex()
            .gap(space::GUTTER)
            .child(nav)
            .child(
                panel(cx).flex_1().child(
                    div()
                        .id("settings-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px(px(34.))
                        .py(px(26.))
                        .child(motion::enter(
                            div()
                                .max_w(px(760.))
                                .flex()
                                .flex_col()
                                .gap(px(18.))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(4.))
                                        .child(title(heading).text_size(px(20.)).line_height(px(26.)))
                                        .child(sub(blurb, cx)),
                                )
                                .child(body),
                            ("settings-page", page as usize),
                            motion::BASE,
                            6.,
                        )),
                ),
            )
            .into_any_element()
    }

    fn dropdown(
        &self,
        key: &'static str,
        options: &[&'static str],
        selected: usize,
        cx: &mut Context<Self>,
        on_select: impl Fn(&mut Workspace, usize, &mut Window, &mut Context<Workspace>) + 'static,
    ) -> Dropdown {
        Dropdown::new(
            key,
            Button::new((key, 0usize), options[selected.min(options.len() - 1)]).icon_right(IconName::ChevronDown),
            options.iter().map(|o| MenuItem::new(*o)).collect(),
        )
        .selected(selected)
        .open(self.menu_open(key))
        .on_toggle(with_value(cx, move |this, open: bool, _, cx| this.toggle_menu(key, open, cx)))
        .on_select(with_value(cx, move |this, i: usize, w, cx| {
            on_select(this, i, w, cx);
            this.save_prefs();
            cx.notify();
        }))
    }

    fn toggle(&self, id: &'static str, on: bool, cx: &mut Context<Self>, set: fn(&mut Workspace, bool)) -> Toggle {
        Toggle::new(id, on).on_toggle(with_value(cx, move |this, v: bool, _, cx| {
            set(this, v);
            this.save_prefs();
            cx.notify();
        }))
    }

    fn settings_general(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mode_ix = ThemeMode::ALL.iter().position(|m| *m == self.prefs.theme).unwrap_or(0);
        let appearance = Segmented::new(
            "theme-mode",
            vec![
                Segment::new("System"),
                Segment::new("Light").icon(IconName::Sun),
                Segment::new("Dark").icon(IconName::Moon),
            ],
            mode_ix,
        )
        .on_select(with_value(cx, |this, i: usize, w, cx| this.set_theme(ThemeMode::ALL[i], w, cx)));
        let t = cx.theme().clone();
        let swatches = div().flex().gap(px(8.)).children(Palette::ALL.iter().enumerate().map(|(i, p)| {
            let p = *p;
            let on = p == self.prefs.palette;
            let (bg, accent) = p.swatch(t.appearance);
            div()
                .id(("palette", i))
                .size(px(26.))
                .rounded_full()
                .p(px(2.))
                .border_2()
                .border_color(if on { t.fg_2 } else { gpui::transparent_black() })
                .cursor_pointer()
                .hover_style(|s| s.border_color(t.line_strong))
                .on_click(cx.listener(move |this, _, w, cx| this.set_palette(p, w, cx)))
                .child(
                    div()
                        .size_full()
                        .rounded_full()
                        .bg(bg)
                        .border_1()
                        .border_color(t.line_strong)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().size(px(9.)).rounded_full().bg(accent)),
                )
        }));
        let palette_desc = format!("{}. Tints the app chrome and its accent.", self.prefs.palette.label());
        let auto_render = self.toggle("auto-render", self.prefs.auto_render, cx, |w, v| w.prefs.auto_render = v);
        let setup = Button::new("reopen-setup", "Open setup").on_click(click(cx, |this, _, cx| {
            this.onboarding_step = 0;
            this.go(Screen::Onboarding, cx);
        }));
        group(cx)
            .child(item("Appearance", "Match your system, or pick light or dark.", appearance, false, cx))
            .child(item("Colour", palette_desc, swatches, false, cx))
            .child(item("Render when the assistant finishes", "Finished scripts go straight into the queue.", auto_render, false, cx))
            .child(item("Setup guide", "Reconnect assistants or pick a different house style.", setup, true, cx))
            .into_any_element()
    }

    fn settings_assistants(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let cards = self.providers.clone().into_iter().enumerate().map(|(i, p)| {
            let is_default = p.provider == self.default_provider;
            let bar = |label: &'static str, v: Option<f32>| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(px(12.))
                            .child(div().text_color(t.fg_2).child(label))
                            .child(num(v.map_or("—".into(), |v| format!("{}%", (v * 100.).round() as u32)))),
                    )
                    .child(ProgressBar::new(v.unwrap_or(0.)).tone(Tone::Neutral).height(6.))
            };
            let provider = p.provider;
            div()
                .id(("usage", i))
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(14.))
                .rounded(radius::PANEL)
                .bg(t.surface_2)
                .border_1()
                .border_color(t.line)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(provider_mark(p.provider.icon(), 28., cx))
                        .child(strong(match (&p.plan, p.signed_in) {
                            (Some(plan), true) => format!("{} · {plan}", p.provider.name()),
                            _ => p.provider.name().to_string(),
                        }))
                        .child(div().flex_1())
                        .child(if p.checking && !p.installed {
                            Chip::new("Checking…").into_any_element()
                        } else if !p.installed {
                            Button::new(("install", i), "Install")
                                .small()
                                .on_click(click(cx, move |this, _, cx| this.install_provider(provider, cx)))
                                .into_any_element()
                        } else if !p.signed_in {
                            Button::new(("sign-in", i), "Sign in")
                                .small()
                                .on_click(click(cx, move |this, _, cx| this.sign_in_provider(provider, cx)))
                                .into_any_element()
                        } else if is_default {
                            Chip::new("Default").tone(Tone::Ok).into_any_element()
                        } else {
                            Button::new(("make-default", i), "Make default")
                                .small()
                                .on_click(click(cx, move |this, _, cx| {
                                    this.set_default_provider(provider);
                                    cx.notify();
                                }))
                                .into_any_element()
                        }),
                )
                .child(bar("5-hour window", p.window_used))
                .child(bar("Weekly", p.week_used))
                .child(
                    faint(
                        match (p.resets_in(), p.window_used) {
                            (Some(r), _) => format!("Window resets in {r}"),
                            (None, None) if p.signed_in => "Usage appears after the first turn.".to_string(),
                            (None, _) => format!("{} {}", p.provider.cli(), p.version.clone().unwrap_or_default()),
                        },
                        cx,
                    )
                    .text_size(text::CAPS),
                )
        }).collect::<Vec<_>>();
        let p = self.default_provider;
        let planning = pair(
            self.tier_menu("planning-model", self.prefs.planning_tier, p, false, false, cx, |pr, v| pr.planning_tier = v),
            self.reasoning_menu("planning-reasoning", self.prefs.planning_reasoning, false, cx, |pr, v| pr.planning_reasoning = v),
        );
        let build = pair(
            self.tier_menu("build-model", self.prefs.build_tier, p, false, false, cx, |pr, v| pr.build_tier = v),
            self.reasoning_menu("build-reasoning", self.prefs.build_reasoning, false, cx, |pr, v| pr.build_reasoning = v),
        );
        let switch = self.toggle("auto-switch", self.prefs.auto_switch_provider, cx, |w, v| w.prefs.auto_switch_provider = v);
        let lean = self.toggle("lean", self.prefs.lean_context, cx, |w, v| w.prefs.lean_context = v);
        let escalate = self.toggle("escalate", self.prefs.escalate_repairs, cx, |w, v| w.prefs.escalate_repairs = v);
        let usage = self.toggle("show-usage", self.prefs.show_usage, cx, |w, v| w.prefs.show_usage = v);
        let repairs = self.dropdown(
            "repairs",
            &["Off", "1 try", "2 tries", "3 tries"],
            self.prefs.repair_tries as usize,
            cx,
            |w, i, _, _| w.prefs.repair_tries = i as u8,
        );
        let budgets: Vec<&'static str> = TURN_BUDGETS.iter().map(|(l, _)| *l).collect();
        let budget = self.dropdown("turn-budget", &budgets, self.prefs.turn_budget, cx, |w, i, _, _| w.prefs.turn_budget = i);
        let is_recommended = self.prefs.ai_is_recommended();
        let reset = if is_recommended {
            Chip::new("In use").tone(Tone::Ok).icon(IconName::Check).into_any_element()
        } else {
            Button::new("ai-reset", "Use recommended")
                .on_click(click(cx, |this, _, cx| {
                    this.prefs.reset_ai();
                    this.save_prefs();
                    cx.notify();
                }))
                .into_any_element()
        };
        let auto_note = "Auto thinks lightly for questions and small edits, and more for new scenes and repairs.";
        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(div().flex().gap(px(10.)).children(cards))
            .child(section("Models and reasoning", cx))
            .child(
                group(cx)
                    .child(item("Planning", "Storyboards and questions. A small model with Auto reasoning is enough.", planning, false, cx))
                    .child(item("Building", "Writes and repairs scene code. Also set from the chat box. Auto reasoning builds with low effort on Sonnet (fewer tokens, fewer repairs in our tests).", build, false, cx))
                    .child(item("Automatic repairs", "Script errors go back to the assistant.", repairs, false, cx))
                    .child(item("Think harder on repeat errors", "Each failed repair raises reasoning one step, then it drops back.", escalate, true, cx)),
            )
            .child(faint(auto_note, cx).text_size(text::CAPS).mt(px(-10.)).px(px(2.)))
            .child(section("Token use", cx))
            .child(
                group(cx)
                    .child(item("Lean context", "Sends the API outline and only the scene being edited, never the whole project.", lean, false, cx))
                    .child(item("Ask before a big turn", "Pauses and asks if one turn would use more than this.", budget, false, cx))
                    .child(item("Show usage after each turn", "A short line in the chat with tokens and plan used.", usage, false, cx))
                    .child(item("Switch assistant when a plan runs low", "Continues on the other connected plan instead of pausing the queue.", switch, false, cx))
                    .child(item("Recommended settings", "Haiku plans, Sonnet builds, reasoning on Auto, 25k turn limit. Best quality per token.", reset, true, cx)),
            )
            .into_any_element()
    }

    /// Model-size picker, labelled with the default assistant's model names.
    #[allow(clippy::too_many_arguments)]
    pub fn tier_menu(
        &self,
        key: &'static str,
        value: Tier,
        provider: Provider,
        compact: bool,
        show_logo: bool,
        cx: &mut Context<Self>,
        set: fn(&mut Prefs, Tier),
    ) -> Dropdown {
        let trigger = Button::new((key, 0usize), value.model(provider)).icon_right(IconName::ChevronDown);
        let trigger = if compact { trigger.ghost().small() } else { trigger };
        let trigger = if show_logo { trigger.leading(Icon::new(provider.icon()).small()) } else { trigger };
        let items = Tier::ALL.iter().map(|t| MenuItem::new(t.model(provider)).detail(t.detail())).collect();
        self.picker(key, trigger, items, value.ix(), compact, cx, move |w, i| set(&mut w.prefs, Tier::ALL[i]))
    }

    pub fn reasoning_menu(
        &self,
        key: &'static str,
        value: Reasoning,
        compact: bool,
        cx: &mut Context<Self>,
        set: fn(&mut Prefs, Reasoning),
    ) -> Dropdown {
        let trigger = Button::new((key, 0usize), value.label()).icon_right(IconName::ChevronDown);
        let trigger = if compact {
            trigger.ghost().small().leading(Icon::new(IconName::Gauge).small()).tooltip("Reasoning")
        } else {
            trigger.with_icon(IconName::Gauge)
        };
        let items = Reasoning::ALL.iter().map(|r| MenuItem::new(r.label()).detail(r.detail())).collect();
        self.picker(key, trigger, items, value.ix(), compact, cx, move |w, i| set(&mut w.prefs, Reasoning::ALL[i]))
    }

    #[allow(clippy::too_many_arguments)]
    fn picker(
        &self,
        key: &'static str,
        trigger: Button,
        items: Vec<MenuItem>,
        selected: usize,
        upward: bool,
        cx: &mut Context<Self>,
        on_select: impl Fn(&mut Workspace, usize) + 'static,
    ) -> Dropdown {
        Dropdown::new(key, trigger, items)
            .selected(selected)
            .width(280.)
            .when(upward, |d| d.upward())
            .open(self.menu_open(key))
            .on_toggle(with_value(cx, move |this, open: bool, _, cx| this.toggle_menu(key, open, cx)))
            .on_select(with_value(cx, move |this, i: usize, _, cx| {
                on_select(this, i);
                this.save_prefs();
                cx.notify();
            }))
    }

    fn settings_styles(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let cards: Vec<_> = HOUSE_STYLES
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let on = i == self.prefs.house_style;
                div()
                    .id(("style-card", i))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(6.))
                    .pb(px(10.))
                    .rounded(radius::PANEL)
                    .bg(if on { t.surface_3 } else { t.surface_2 })
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
                    .child(div().w_full().aspect_ratio(16. / 9.).rounded(radius::THUMB).overflow_hidden().child(crate::app::style_image(self, i, s.art)))
                    .child(
                        div()
                            .px(px(4.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().min_w_0().flex().flex_col().child(strong(s.name)).child(faint(s.blurb, cx)))
                            .when(on, |d| d.child(div().flex_none().child(Chip::new("Default").tone(Tone::Ok)))),
                    )
            })
            .collect();
        let mut grid = div().flex().flex_col().gap(px(10.));
        let mut it = cards.into_iter();
        while let Some(a) = it.next() {
            grid = grid.child(div().flex().gap(px(10.)).child(a).children(it.next()));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(grid)
            .into_any_element()
    }

    fn settings_voice(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use crate::engine::{CAPTIONS, SPEEDS};
        let narration = Toggle::new("narration", self.prefs.narration).on_toggle(with_value(cx, |this, v: bool, _, cx| {
            this.prefs.narration = v;
            this.save_prefs();
            this.refresh_voice(this.current, cx);
            cx.notify();
        }));
        use studio_render::{kokoro, voice::SYSTEM_DEFAULT};
        let ready = self.natural == crate::engine::NaturalVoice::Ready;
        // Shown names and the `prefs.voice` value each one stands for.
        let mut names: Vec<String> = Vec::new();
        let mut values: Vec<Option<String>> = Vec::new();
        if ready {
            for (id, label) in kokoro::VOICES {
                names.push(label.to_string());
                values.push(Some(id.to_string()));
            }
        }
        names.push("System default".into());
        values.push(if ready { Some(SYSTEM_DEFAULT.into()) } else { None });
        for v in &self.voices {
            names.push(v.clone());
            values.push(Some(v.clone()));
        }
        let current = match &self.prefs.voice {
            None if ready => Some(kokoro::DEFAULT_VOICE.to_string()),
            Some(v) if !ready && (v == SYSTEM_DEFAULT || kokoro::is_voice(v)) => None,
            v => v.clone(),
        };
        let selected = values.iter().position(|v| *v == current).unwrap_or(0);
        let voice = Dropdown::new(
            "voice",
            Button::new(("voice", 0usize), names[selected.min(names.len() - 1)].clone()).icon_right(IconName::ChevronDown),
            names.iter().map(|n| MenuItem::new(n.clone())).collect(),
        )
        .selected(selected)
        .width(260.)
        .open(self.menu_open("voice"))
        .on_toggle(with_value(cx, |this, open: bool, _, cx| this.toggle_menu("voice", open, cx)))
        .on_select(with_value(cx, move |this, i: usize, _, cx| {
            this.open_menu = None;
            this.prefs.voice = values.get(i).cloned().flatten();
            this.save_prefs();
            this.refresh_voice(this.current, cx);
            cx.notify();
        }));
        let speed = self.dropdown("speed", &SPEEDS.map(|(n, _)| n), self.prefs.speed, cx, |this, i, _, cx| {
            this.prefs.speed = i;
            this.refresh_voice(this.current, cx);
        });
        let captions = self.dropdown("captions", &CAPTIONS, self.prefs.captions, cx, |this, i, _, _| this.prefs.captions = i);
        let natural: AnyElement = match &self.natural {
            crate::engine::NaturalVoice::Ready => faint("Installed", cx).into_any_element(),
            crate::engine::NaturalVoice::Downloading(f) => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(120.)).child(ProgressBar::new(*f).tone(Tone::Neutral).height(6.)))
                .child(faint(format!("{:.0}%", f * 100.), cx))
                .into_any_element(),
            crate::engine::NaturalVoice::Missing | crate::engine::NaturalVoice::Failed(_) => {
                let size = studio_render::voice::kokoro_dir().map_or(0, |d| kokoro::download_size(d, self.voice_quality()));
                let label = if matches!(self.natural, crate::engine::NaturalVoice::Failed(_)) {
                    "Try again".to_string()
                } else {
                    format!("Download · {} MB", size / 1_000_000)
                };
                Button::new("natural-voice", label)
                    .with_icon(IconName::Download)
                    .on_click(click(cx, |this, _, cx| this.download_natural_voice(cx)))
                    .into_any_element()
            }
        };
        let natural_desc = match &self.natural {
            crate::engine::NaturalVoice::Failed(e) => format!("Download failed: {e}"),
            _ => "Kokoro: lifelike voices that run on this computer. One-time download; nothing is sent anywhere.".into(),
        };
        const QUALITIES: [&str; 2] = ["Standard · 92 MB", "High · 326 MB"];
        let quality = self.dropdown("voice-quality", &QUALITIES, self.prefs.voice_model, cx, |this, i, _, cx| this.set_voice_quality(i, cx));
        let quality_desc = if ready && !self.voice_quality_installed() && !matches!(self.natural, crate::engine::NaturalVoice::Downloading(_)) {
            "Not downloaded yet; the other model speaks until it is. Pick it again to download."
        } else {
            "Standard is a smaller 8-bit copy of the original High model and sounds the same to most ears. Only the one you pick is kept on disk; switching re-speaks the narration."
        };
        use crate::engine::NaturalVoice as Dl;
        let espeak_found = studio_render::voice::espeak();
        let espeak: AnyElement = match &self.espeak {
            Dl::Ready => faint(if espeak_found.is_some_and(|e| !e.bundled) { "Found on this computer" } else { "Installed" }, cx)
                .into_any_element(),
            Dl::Downloading(f) => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(120.)).child(ProgressBar::new(*f).tone(Tone::Neutral).height(6.)))
                .child(faint(format!("{:.0}%", f * 100.), cx))
                .into_any_element(),
            Dl::Missing | Dl::Failed(_) if studio_render::kokoro::espeak::CAN_INSTALL => {
                let label = if matches!(self.espeak, Dl::Failed(_)) {
                    "Try again".to_string()
                } else {
                    format!("Download · {} MB", studio_render::kokoro::espeak::DOWNLOAD_SIZE / 1_000_000)
                };
                Button::new("espeak", label)
                    .with_icon(IconName::Download)
                    .on_click(click(cx, |this, _, cx| this.install_espeak(cx)))
                    .into_any_element()
            }
            Dl::Missing | Dl::Failed(_) => faint(
                if cfg!(target_os = "macos") { "brew install espeak-ng" } else { "sudo apt install espeak-ng" },
                cx,
            )
            .into_any_element(),
        };
        let espeak_desc = match &self.espeak {
            Dl::Failed(e) => format!("Download failed: {e}"),
            _ => "espeak-ng reads names and words the natural voice's dictionaries don't know; without it they are spelled out letter by letter. Free (GPL), runs as a separate program.".into(),
        };
        let preview = Button::new("voice-try", "Listen").with_icon(IconName::Play).on_click(click(cx, |this, _, cx| this.preview_voice(cx)));
        group(cx)
            .child(item("Narration", "Read each scene's narration aloud and time the scene to it.", narration, false, cx))
            .child(item("Natural voice", natural_desc, natural, false, cx))
            .child(item("Voice quality", quality_desc, quality, false, cx))
            .child(item("Unusual words", espeak_desc, espeak, false, cx))
            .child(item(
                "Voice",
                if ready {
                    "Natural voices first, then your computer's built-in ones. Each line is spoken once and cached."
                } else {
                    "Your computer's built-in voices until the natural voice is downloaded. Each line is spoken once and cached."
                },
                div().flex().gap(px(6.)).child(preview).child(voice),
                false,
                cx,
            ))
            .child(item("Speed", "Changing it re-speaks the narration.", speed, false, cx))
            .child(item("Captions", "Timed from the narration. Burned-in captions are drawn on the video; the .srt file is for YouTube, LMS uploads and screen readers.", captions, true, cx))
            .into_any_element()
    }

    fn settings_rendering(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use crate::engine::{BACKENDS, ENCODERS, OUTPUTS};
        let output = self.dropdown("output", &OUTPUTS.map(|(n, _)| n), self.prefs.output, cx, |this, i, _, _| this.prefs.output = i);
        let renderer = self.dropdown("renderer", &BACKENDS.map(|(n, _)| n), self.prefs.renderer, cx, |this, i, _, _| this.prefs.renderer = i);
        let encoder = self.dropdown("encoder", &ENCODERS.map(|(n, _)| n), self.prefs.encoder, cx, |this, i, _, _| this.prefs.encoder = i);
        use crate::engine::NaturalVoice as Dl;
        let ffmpeg: AnyElement = match &self.ffmpeg_dl {
            Dl::Downloading(f) => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(120.)).child(ProgressBar::new(*f).tone(Tone::Neutral).height(6.)))
                .child(faint(format!("{:.0}%", f * 100.), cx))
                .into_any_element(),
            _ if self.engine.ffmpeg => Chip::new("Ready").tone(Tone::Ok).icon(IconName::Check).into_any_element(),
            Dl::Failed(_) => Button::new("ffmpeg-dl", "Try again")
                .with_icon(IconName::Download)
                .on_click(click(cx, |this, _, cx| this.download_ffmpeg(cx)))
                .into_any_element(),
            _ => Button::new("ffmpeg-dl", format!("Download · {} MB", studio_render::tools::ffmpeg_download_size() / 1_000_000))
                .with_icon(IconName::Download)
                .on_click(click(cx, |this, _, cx| this.download_ffmpeg(cx)))
                .into_any_element(),
        };
        let ffmpeg_desc = match &self.ffmpeg_dl {
            Dl::Failed(e) => format!("The download failed: {e}"),
            Dl::Downloading(_) => "Downloading ffmpeg for MP4 export. You can keep working; renders start once it's ready.".to_string(),
            _ if studio_render::tools::ffmpeg().is_some() => format!(
                "Writes MP4 and joins scenes. Studio's own copy (ffmpeg {}, GPL, licence saved beside it).",
                studio_render::tools::FFMPEG_VERSION
            ),
            _ if self.engine.ffmpeg => "Writes MP4 and joins scenes. Using the ffmpeg installed on this computer.".to_string(),
            _ => "Needed to write MP4 and join scenes. Studio downloads it once (no installer, no admin rights).".to_string(),
        };
        let render = group(cx)
            .child(item("Output", "Resolution and frame rate of rendered videos. Scenes rendered at another size render again.", output, false, cx))
            .child(item("Renderer", "Automatic times GPU and CPU on your scene and keeps the faster.", renderer, false, cx))
            .child(item("Encoder", "Hardware encoding is much faster on most graphics cards.", encoder, false, cx))
            .child(item("ffmpeg", ffmpeg_desc, ffmpeg, true, cx));
        div().flex().flex_col().gap(px(18.)).child(render).child(self.settings_watermark(cx)).into_any_element()
    }

    fn settings_watermark(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use crate::engine::{WM_COLORS, WM_MODES, WM_RANGES};
        let w = self.prefs.watermark.clone();
        let mode = Segmented::new("wm-mode", WM_MODES.iter().map(|m| Segment::new(*m)).collect(), w.mode.min(2)).on_select(with_value(
            cx,
            |this, i: usize, _, cx| {
                if i == 2 && this.prefs.watermark.image.as_ref().is_none_or(|p| !p.is_file()) {
                    this.pick_watermark_image(cx);
                    return;
                }
                this.prefs.watermark.mode = i;
                this.save_prefs();
                cx.notify();
            },
        ));
        let off_here = self.projects.get(self.current.0).filter(|p| p.no_watermark).map(|p| p.name.clone());
        let mode_desc = match off_here {
            Some(name) if w.mode > 0 => format!("Drawn on every exported video. Turned off for “{name}” (scene strip → Watermark)."),
            _ => "Drawn on every exported video, not on the preview. Turn it off per project from the scene strip.".to_string(),
        };
        let mut g = group(cx).child(item("Watermark", mode_desc, mode, w.mode == 0, cx));
        if w.mode == 0 {
            return g.into_any_element();
        }
        let t = cx.theme().clone();
        let is_text = w.mode == 1;

        // Mark content.
        let content: AnyElement = if is_text {
            div()
                .w(px(280.))
                .px(px(10.))
                .py(px(6.))
                .rounded(radius::CONTROL)
                .bg(t.surface)
                .border_1()
                .border_color(t.line_strong)
                .cursor_text()
                .on_mouse_down(gpui::MouseButton::Left, crate::app::focus_on_click(&self.watermark_input))
                .child(self.watermark_input.clone())
                .into_any_element()
        } else {
            let name = w.image.as_ref().filter(|p| p.is_file()).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .when_some(name.clone(), |d, n| d.child(faint(n, cx)))
                .child(
                    Button::new("wm-image", if name.is_some() { "Change…" } else { "Choose image…" })
                        .with_icon(IconName::Image)
                        .on_click(click(cx, |this, _, cx| this.pick_watermark_image(cx))),
                )
                .into_any_element()
        };

        // Position: a 3×3 grid of spots.
        let mut anchor = div().flex().flex_col().gap(px(3.));
        for row in 0..3 {
            let mut r = div().flex().gap(px(3.));
            for col in 0..3 {
                let ix = row * 3 + col;
                let on = w.anchor.min(8) == ix;
                r = r.child(
                    div()
                        .id(("wm-anchor", ix))
                        .w(px(26.))
                        .h(px(16.))
                        .rounded(radius::INNER)
                        .border_1()
                        .border_color(if on { t.primary } else { t.line_strong })
                        .bg(if on { t.primary } else { t.surface })
                        .cursor_pointer()
                        .on_click(click(cx, move |this, _, cx| {
                            this.prefs.watermark.anchor = ix;
                            this.save_prefs();
                            cx.notify();
                        })),
                );
            }
            anchor = anchor.child(r);
        }

        let slider = |ix: usize, value: f32, label: String, cx: &mut Context<Self>| {
            let (lo, hi) = WM_RANGES[ix];
            let f = ((value - lo) / (hi - lo)).clamp(0., 1.);
            let bounds = self.wm_bounds[ix].clone();
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .id(("wm-slider", ix))
                        .relative()
                        .w(px(200.))
                        .h(px(20.))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .child(gpui::canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full())
                        .child(div().w_full().h(px(4.)).rounded(radius::BAR).bg(t.surface_3).child(div().h_full().rounded(radius::BAR).bg(t.primary).w(gpui::relative(f))))
                        .child(
                            div()
                                .absolute()
                                .left(gpui::relative(f))
                                .ml(px(-7.))
                                .size(px(14.))
                                .rounded_full()
                                .bg(gpui::white())
                                .border_2()
                                .border_color(t.primary),
                        )
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                                this.wm_drag = Some(ix);
                                this.set_wm_slider(ix, ev.position.x, cx);
                            }),
                        )
                        .on_mouse_move(cx.listener(move |this, ev: &gpui::MouseMoveEvent, _, cx| {
                            if this.wm_drag == Some(ix) {
                                if ev.pressed_button == Some(gpui::MouseButton::Left) {
                                    this.set_wm_slider(ix, ev.position.x, cx);
                                } else {
                                    this.wm_drag = None;
                                    this.save_prefs();
                                }
                            }
                        }))
                        .on_mouse_up(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, _| {
                                this.wm_drag = None;
                                this.save_prefs();
                            }),
                        ),
                )
                .child(div().w(px(44.)).text_size(studio_ui::theme::text::SMALL).text_color(t.fg_2).child(label))
        };
        let size = slider(0, w.scale, format!("{:.0}%", w.scale * 100.), cx);
        let opacity = slider(1, w.alpha, format!("{:.0}%", w.alpha * 100.), cx);
        let margin = slider(2, w.margin, format!("{:.1}%", w.margin * 100.), cx);

        g = g.child(div().px(px(14.)).py(px(14.)).border_b_1().border_color(t.line).flex().justify_center().child(self.watermark_preview(cx)));
        g = g.child(item(if is_text { "Text" } else { "Image" }, if is_text { "Your name, school or channel. Saved as you type." } else { "A PNG with a transparent background looks best. Studio keeps its own copy." }, content, false, cx));
        g = g
            .child(item("Position", "Corners, edges or the centre.", anchor, false, cx))
            .child(item("Size", "Relative to the video, so it looks the same at every resolution.", size, false, cx))
            .child(item("Opacity", "Lower is subtler.", opacity, false, cx))
            .child(item("Margin", "Distance from the edges.", margin, !is_text, cx));
        if is_text {
            let color = Segmented::new("wm-color", WM_COLORS.iter().map(|(n, _)| Segment::new(*n)).collect(), w.color.min(2))
                .on_select(with_value(cx, |this, i: usize, _, cx| {
                    this.prefs.watermark.color = i;
                    this.save_prefs();
                    cx.notify();
                }));
            let shadow = self.toggle("wm-shadow", w.shadow, cx, |ws, v| ws.prefs.watermark.shadow = v);
            g = g
                .child(item("Colour", "Style uses the video's house style text colour.", color, false, cx))
                .child(item("Shadow", "Keeps text readable over busy frames.", shadow, true, cx));
        }
        g.into_any_element()
    }

    /// How the watermark will sit on the current video (same geometry as the export).
    fn watermark_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::engine::{WM_COLORS, WM_IMAGE_WIDTH, WM_TEXT_HEIGHT};
        const W: f32 = 480.;
        const H: f32 = 270.;
        let t = cx.theme().clone();
        let w = &self.prefs.watermark;
        let (style_text, style_bg) = self.style_rgb();
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap_or(0xFFFFFF);
        let rgba = |s: &str, a: f32| gpui::rgba((hex(s) << 8) | ((a.clamp(0., 1.) * 255.) as u32));
        let video = self.video();
        let background: AnyElement = match video.scenes.iter().find(|s| s.engine.thumb.is_some()) {
            Some(s) => crate::app::scene_image(s, cx),
            None => crate::app::style_image(self, self.video_style(self.current), studio_ui::assets::Art::Circle),
        };
        let a = w.anchor.min(8);
        let m = W.min(H) * w.margin;
        let mark: AnyElement = match w.mode {
            1 => {
                let text = if w.text.trim().is_empty() { "Your name here".to_string() } else { w.text.trim().to_string() };
                let size = H * WM_TEXT_HEIGHT * w.scale;
                let color = WM_COLORS[w.color.min(2)].1.map(str::to_string).unwrap_or(style_text);
                let d = (size / 18.).max(1.);
                let label = |c: gpui::Rgba| {
                    div().text_size(px(size)).line_height(px(size * 1.2)).font_weight(gpui::FontWeight::SEMIBOLD).whitespace_nowrap().text_color(c).child(text.clone())
                };
                div()
                    .relative()
                    .when(w.shadow, |e| e.child(label(rgba(&style_bg, w.alpha * 0.6)).absolute().left(px(d)).top(px(d))))
                    .child(label(rgba(&color, w.alpha)))
                    .into_any_element()
            }
            _ => match w.image.as_ref().filter(|p| p.is_file()) {
                Some(p) => gpui::img(p.clone()).w(px(W * WM_IMAGE_WIDTH * w.scale)).opacity(w.alpha).into_any_element(),
                None => div().into_any_element(),
            },
        };
        let holder = div().absolute().top_0().left_0().size_full().p(px(m)).flex();
        let holder = match a % 3 {
            0 => holder.justify_start(),
            1 => holder.justify_center(),
            _ => holder.justify_end(),
        };
        let holder = match a / 3 {
            0 => holder.items_start(),
            1 => holder.items_center(),
            _ => holder.items_end(),
        };
        div()
            .relative()
            .w(px(W))
            .h(px(H))
            .rounded(radius::CHIP)
            .overflow_hidden()
            .border_1()
            .border_color(t.line)
            .bg(t.stage)
            .child(background)
            .child(holder.child(mark))
    }

    fn settings_updates(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let channel = Segmented::new("channel", vec![Segment::new("Stable"), Segment::new("Beta")], self.prefs.update_channel)
            .on_select(with_value(cx, |this, i: usize, _, cx| {
                this.prefs.update_channel = i;
                this.save_prefs();
                cx.notify();
            }));
        use crate::update::UpdateState;
        let v = crate::update::VERSION;
        let status = match &self.update {
            UpdateState::Disabled => format!("Manim Studio {v} · this build isn't set up for updates"),
            UpdateState::Idle => format!("Manim Studio {v}"),
            UpdateState::Checking => format!("Manim Studio {v} · checking…"),
            UpdateState::UpToDate => format!("Manim Studio {v} · up to date"),
            UpdateState::Downloading(n) => format!("Manim Studio {v} · downloading {n}…"),
            UpdateState::Ready { version, .. } => format!("Manim Studio {v} · {version} is ready, restart to install"),
            UpdateState::Error(e) => format!("Manim Studio {v} · couldn't check: {e}"),
        };
        let ready = matches!(self.update, UpdateState::Ready { .. });
        let disabled = self.update == UpdateState::Disabled;
        let check = if ready {
            Button::new("restart-update", "Restart now").primary().on_click(click(cx, |this, _, cx| this.restart_to_update(cx)))
        } else {
            Button::new("check-updates", "Check now")
                .with_icon(IconName::Refresh)
                .disabled(disabled || self.update == UpdateState::Checking)
                .on_click(click(cx, |this, _, cx| this.check_for_update(cx)))
        };
        let rollback = self.can_roll_back();
        group(cx)
            .child(item("Version", status, check, false, cx))
            .child(item("Channel", "Beta gets new features a week early.", channel, false, cx))
            .child(item(
                "Verified updates",
                "Every update is signed and checked before it is installed. The previous version is kept for rollback.",
                if disabled {
                    Chip::new("Off in this build").into_any_element()
                } else {
                    Chip::new("On").tone(Tone::Ok).icon(IconName::Shield).into_any_element()
                },
                !rollback,
                cx,
            ))
            .when(rollback, |d| {
                d.child(item(
                    "Previous version",
                    "Go back to the version you had before the last update.",
                    Button::new("rollback", "Roll back").ghost().on_click(click(cx, |this, _, cx| this.roll_back(cx))),
                    true,
                    cx,
                ))
            })
            .into_any_element()
    }

    fn settings_shortcuts(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let all = crate::shortcuts::list();
        let mut groups: Vec<(&'static str, Vec<crate::shortcuts::Shortcut>)> = Vec::new();
        for s in all {
            match groups.iter_mut().find(|(g, _)| *g == s.group) {
                Some((_, v)) => v.push(s),
                None => groups.push((s.group, vec![s])),
            }
        }
        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .children(groups.into_iter().map(|(name, rows)| {
                let n = rows.len();
                div().flex().flex_col().gap(px(8.)).child(caps(name, cx)).child(group(cx).children(
                    rows.into_iter().enumerate().map(|(i, s)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .px(px(14.))
                            .h(px(40.))
                            .when(i + 1 < n, |d| d.border_b_1().border_color(t.line))
                            .child(div().flex_1().min_w_0().text_color(t.fg).child(s.label))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .children(crate::shortcuts::caps(&s.keys).into_iter().map(Kbd::new)),
                            )
                    }),
                ))
            }))
            .into_any_element()
    }

    fn settings_about(&mut self, cx: &mut Context<Self>) -> AnyElement {
        group(cx)
            .child(item("Manim Studio", concat!("Version ", env!("CARGO_PKG_VERSION")), Button::new("release-notes", "Release notes").ghost().with_icon(IconName::External), false, cx))
            .child(item("Open-source licenses", "Inter, JetBrains Mono (OFL) · GPUI-CE (Apache-2.0)", Button::new("licenses", "View").ghost(), false, cx))
            .child(item(
                "Report a problem",
                if crate::crash::latest().is_some() {
                    "Opens a pre-filled issue with the latest crash report's summary. Nothing is sent automatically."
                } else {
                    "Opens a pre-filled issue. Nothing is sent automatically."
                },
                Button::new("report", "Report").ghost().with_icon(IconName::External).on_click(|_, _, _| crate::crash::open_url(&crate::crash::issue_url())),
                true,
                cx,
            ))
            .into_any_element()
    }
}
