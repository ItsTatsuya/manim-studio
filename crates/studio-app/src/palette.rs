//! The command palette (Ctrl K): one box that finds commands and videos. Arrow keys
//! move, Enter runs, Esc or a click outside closes.

use crate::app::{Mode, Screen, SettingsPage, Workspace, faint};
use gpui::{
    ColorExt,
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use studio_ui::components::Kbd;
use studio_ui::motion;
use studio_ui::theme::{ActiveTheme, Palette, ThemeMode, radius, text};
use studio_ui::{Icon, IconName};

/// What a palette row does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Go(Screen),
    Settings(SettingsPage),
    Mode(Mode),
    Render,
    Theme(ThemeMode),
    Colour(Palette),
    ToggleSidebar,
    Setup,
    Video(usize, usize),
}

#[derive(Clone)]
pub struct Item {
    pub command: Command,
    pub label: String,
    /// Right-hand hint: a shortcut (shown as key caps) or a note.
    pub hint: Option<String>,
    pub shortcut: bool,
    pub icon: IconName,
}

/// Rows shown at most.
const MAX_ROWS: usize = 9;

fn item(command: Command, label: impl Into<String>, icon: IconName) -> Item {
    Item { command, label: label.into(), hint: None, shortcut: false, icon }
}

impl Item {
    fn keys(mut self, keys: String) -> Self {
        self.hint = Some(keys);
        self.shortcut = true;
        self
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.hint = Some(note.into());
        self
    }
}

/// Every word of the query appears in the label (or the note), in any order.
fn matches(item: &Item, words: &[String]) -> bool {
    let hay = format!("{} {}", item.label, if item.shortcut { "" } else { item.hint.as_deref().unwrap_or("") }).to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

impl Workspace {
    /// All commands for the current state, in display order (before filtering).
    fn palette_commands(&self) -> Vec<Item> {
        let sec = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
        let video_open = self.screen == Screen::Video;
        let mut v = vec![
            item(Command::Go(Screen::Home), "New video", IconName::Plus).keys(format!("{sec}-n")),
            item(Command::Go(Screen::Renders), "Open renders", IconName::Queue).keys(format!("{sec}-r")),
            item(Command::Go(Screen::Settings), "Open settings", IconName::Gear).keys(format!("{sec}-,")),
        ];
        if video_open {
            v.push(item(Command::Render, "Render the video", IconName::Film).keys(format!("{sec}-enter")));
            v.push(item(Command::Mode(Mode::Storyboard), "Show the storyboard", IconName::Board).keys(format!("{sec}-1")));
            v.push(item(Command::Mode(Mode::Studio), "Show the studio", IconName::Play).keys(format!("{sec}-2")));
            v.push(item(Command::Mode(Mode::Code), "Show the code", IconName::Script).keys(format!("{sec}-3")));
        }
        v.push(item(Command::ToggleSidebar, "Show or hide the sidebar", IconName::Sidebar).keys(format!("{sec}-b")));
        for page in SettingsPage::ALL {
            v.push(item(Command::Settings(page), format!("Settings: {}", page.label()), page.icon()));
        }
        v.push(item(Command::Theme(ThemeMode::System), "Appearance: match system", IconName::Sun));
        v.push(item(Command::Theme(ThemeMode::Light), "Appearance: light", IconName::Sun));
        v.push(item(Command::Theme(ThemeMode::Dark), "Appearance: dark", IconName::Moon));
        for p in Palette::ALL {
            let mut it = item(Command::Colour(p), format!("Colour: {}", p.label()), IconName::Palette);
            if p == self.prefs.palette {
                it = it.note("Current");
            }
            v.push(it);
        }
        v.push(item(Command::Setup, "Open the setup guide", IconName::Spark));
        v
    }

    /// The rows for the current query: videos first when the query names one.
    pub fn palette_items(&self, cx: &gpui::App) -> Vec<Item> {
        let query = self.palette_input.read(cx).text().trim().to_lowercase();
        let words: Vec<String> = query.split_whitespace().map(str::to_string).collect();
        let mut videos: Vec<Item> = self
            .projects
            .iter()
            .enumerate()
            .flat_map(|(pi, p)| {
                p.videos.iter().enumerate().map(move |(vi, v)| {
                    item(Command::Video(pi, vi), v.title.clone(), IconName::Film).note(p.name.clone())
                })
            })
            .collect();
        let mut commands = self.palette_commands();
        if words.is_empty() {
            // Nothing typed: the commands, then the most recent videos.
            videos.truncate(3);
            commands.truncate(MAX_ROWS.saturating_sub(videos.len()));
            commands.extend(videos);
            return commands;
        }
        videos.retain(|i| matches(i, &words));
        commands.retain(|i| matches(i, &words));
        videos.extend(commands);
        videos.truncate(MAX_ROWS);
        videos
    }

    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen == Screen::Onboarding {
            return;
        }
        self.open_menu = None;
        self.row_menu = None;
        self.palette = true;
        self.palette_sel = 0;
        self.palette_input.update(cx, |i, cx| i.clear(cx));
        let handle = gpui::Focusable::focus_handle(self.palette_input.read(cx), cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    pub fn close_palette(&mut self, cx: &mut Context<Self>) {
        if self.palette {
            self.palette = false;
            cx.notify();
        }
    }

    pub fn palette_move(&mut self, by: isize, cx: &mut Context<Self>) {
        let n = self.palette_items(cx).len();
        if n > 0 {
            self.palette_sel = (self.palette_sel as isize + by).rem_euclid(n as isize) as usize;
            cx.notify();
        }
    }

    /// Run the selected row (Enter) or row `index` (click).
    pub fn palette_run(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.palette_items(cx);
        let Some(it) = items.get(index.unwrap_or(self.palette_sel)) else { return };
        let command = it.command;
        self.palette = false;
        window.blur(cx);
        match command {
            Command::Go(s) => self.go(s, cx),
            Command::Settings(page) => {
                self.settings_page = page;
                self.go(Screen::Settings, cx);
            }
            Command::Mode(m) => self.switch_mode(m, cx),
            Command::Render => self.start_render(cx),
            Command::Theme(m) => self.set_theme(m, window, cx),
            Command::Colour(p) => self.set_palette(p, window, cx),
            Command::ToggleSidebar => {
                self.prefs.sidebar_collapsed = !self.prefs.sidebar_collapsed;
                self.save_prefs();
            }
            Command::Setup => {
                self.onboarding_step = 0;
                self.go(Screen::Onboarding, cx);
            }
            Command::Video(p, v) => self.open_video(p, v, cx),
        }
        cx.notify();
    }

    pub fn render_palette(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let items = self.palette_items(cx);
        let sel = self.palette_sel.min(items.len().saturating_sub(1));
        let rows = items.into_iter().enumerate().map(|(i, it)| {
            let on = i == sel;
            div()
                .id(("palette-row", i))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(34.))
                .px(px(10.))
                .rounded(radius::CONTROL)
                .when(on, |d| d.bg(t.surface_3))
                .cursor_pointer()
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.palette_sel != i {
                        this.palette_sel = i;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| this.palette_run(Some(i), window, cx)))
                .child(Icon::new(it.icon).small().color(if on { t.fg } else { t.fg_3 }))
                .child(div().flex_1().min_w_0().truncate().text_color(if on { t.fg } else { t.fg_2 }).child(it.label))
                .when_some(it.hint, |d, h| {
                    if it.shortcut {
                        d.child(
                            div().flex().gap(px(3.)).children(crate::shortcuts::caps(&h).into_iter().map(Kbd::new)),
                        )
                    } else {
                        d.child(div().flex_none().text_size(text::SMALL).text_color(t.fg_3).child(h))
                    }
                })
        });
        let list: Vec<_> = rows.collect();
        let empty = list.is_empty();
        let card = div()
            .id("palette-card")
            .occlude()
            .key_context("Palette")
            .w(px(560.))
            .max_w(gpui::relative(0.9))
            .flex()
            .flex_col()
            .rounded(radius::PANEL)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow(t.pop_shadow())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(t.line)
                    .child(Icon::new(IconName::Search).color(t.fg_3))
                    .child(div().flex_1().min_w_0().child(self.palette_input.clone()))
                    .child(Kbd::new("Esc")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .p(px(6.))
                    .children(list)
                    .when(empty, |d| d.child(div().px(px(10.)).py(px(12.)).child(faint("Nothing matches. Try another word.", cx)))),
            );
        div()
            .id("palette-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .pt(gpui::relative(0.14))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_palette(cx)))
            .child(motion::fade(div().absolute().top_0().left_0().size_full().bg(t.shadow.opacity(0.5)), "palette-dim", motion::FAST))
            .child(motion::rise(card, "palette-rise"))
            .into_any_element()
    }
}
