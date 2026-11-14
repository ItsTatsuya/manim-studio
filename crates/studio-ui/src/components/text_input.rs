//! A soft-wrapping text input used by the composers.
//!
//! Text lives in the entity; one [`ShapedText`] covers the whole value, so caret
//! and selection geometry come straight from GPUI's text layout.

use crate::theme::{ActiveTheme, FONT_UI, text, ui_features};
use gpui::{
    App, AvailableSpace, ColorExt, Bounds, ClipboardItem, Context, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, PaintQuad, ParentElement, Pixels, Point, Render, ShapedText,
    SharedString, Size, Style, Styled, Task, TextAlign, TextRun, UTF16Selection, Window, actions,
    div, font, point, px, relative, size, utf8_to_utf16_offset, utf16_to_utf8_offset,
};
use std::ops::Range;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_input,
    [
        Backspace, Delete, Left, Right, Up, Down, Home, End, SelectLeft, SelectRight, SelectAll,
        Copy, Cut, Paste, Submit, Newline, WordLeft, WordRight, DeleteWordLeft,
    ]
);

const CONTEXT: &str = "TextInput";

pub fn bind_keys(cx: &mut App) {
    let ctx = Some(CONTEXT);
    let secondary = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
    let word = if cfg!(target_os = "macos") { "alt" } else { "ctrl" };
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new(&format!("{secondary}-a"), SelectAll, ctx),
        KeyBinding::new(&format!("{secondary}-c"), Copy, ctx),
        KeyBinding::new(&format!("{secondary}-x"), Cut, ctx),
        KeyBinding::new(&format!("{secondary}-v"), Paste, ctx),
        KeyBinding::new(&format!("{word}-left"), WordLeft, ctx),
        KeyBinding::new(&format!("{word}-right"), WordRight, ctx),
        KeyBinding::new(&format!("{word}-backspace"), DeleteWordLeft, ctx),
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new("shift-enter", Newline, ctx),
    ]);
}

/// Emitted when the user presses Enter with non-empty text.
pub struct Submitted(pub String);

/// Emitted when the input loses focus (inline renames commit on it).
pub struct Blurred;

pub struct TextInput {
    focus_handle: FocusHandle,
    text: String,
    placeholder: SharedString,
    /// Byte range; `selection.end` is the caret.
    selection: Range<usize>,
    anchor: usize,
    marked: Option<Range<usize>>,
    caret_visible: bool,
    blink: Task<()>,
    font_size: Pixels,
    line_height: Pixels,
    min_lines: usize,
    max_lines: usize,
    layout: Option<(ShapedText, Bounds<Pixels>)>,
}

impl EventEmitter<Submitted> for TextInput {}
impl EventEmitter<Blurred> for TextInput {}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        cx.on_focus(&focus_handle, window, |this, _, cx| this.restart_blink(cx))
            .detach();
        cx.on_blur(&focus_handle, window, |this, _, cx| {
            this.caret_visible = false;
            this.blink = Task::ready(());
            cx.emit(Blurred);
            cx.notify();
        })
        .detach();
        Self {
            focus_handle,
            text: String::new(),
            placeholder: placeholder.into(),
            selection: 0..0,
            anchor: 0,
            marked: None,
            caret_visible: false,
            blink: Task::ready(()),
            font_size: text::BODY,
            line_height: px(20.),
            min_lines: 1,
            max_lines: 8,
            layout: None,
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        let end = self.text.len();
        self.selection = end..end;
        self.anchor = end;
        self
    }

    pub fn font_size(mut self, size: Pixels, line_height: Pixels) -> Self {
        self.font_size = size;
        self.line_height = line_height;
        self
    }

    pub fn lines(mut self, min: usize, max: usize) -> Self {
        self.min_lines = min.max(1);
        self.max_lines = max.max(self.min_lines);
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.text = text.into();
        let end = self.text.len();
        self.select_to(end, false);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    /// Replace the text and select all of it (ready to type over, as in a rename).
    pub fn set_text_selected(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.text = text.into();
        self.anchor = 0;
        self.selection = 0..self.text.len();
        cx.notify();
    }

    fn caret(&self) -> usize {
        self.selection.end
    }

    fn ordered(&self) -> Range<usize> {
        let (a, b) = (self.selection.start, self.selection.end);
        a.min(b)..a.max(b)
    }

    fn select_to(&mut self, offset: usize, extend: bool) {
        let offset = offset.min(self.text.len());
        if !extend {
            self.anchor = offset;
        }
        self.selection = self.anchor..offset;
    }

    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        self.caret_visible = true;
        self.blink = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(530)).await;
                if this
                    .update(cx, |this, cx| {
                        this.caret_visible = !this.caret_visible;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, new: &str, cx: &mut Context<Self>) {
        self.text.replace_range(range.clone(), new);
        let caret = range.start + new.len();
        self.select_to(caret, false);
        self.marked = None;
        self.restart_blink(cx);
    }

    fn prev_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .find_map(|(i, _)| (i > offset).then_some(i))
            .unwrap_or(self.text.len())
    }

    fn word_start(&self, offset: usize) -> usize {
        let before = &self.text[..offset];
        let trimmed = before.trim_end();
        trimmed
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8())
    }

    fn word_end(&self, offset: usize) -> usize {
        let after = &self.text[offset..];
        let skip = after.len() - after.trim_start().len();
        let rest = &after[skip..];
        offset + skip + rest.find(char::is_whitespace).unwrap_or(rest.len())
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.word_start(self.caret());
        self.select_to(to, false);
        self.restart_blink(cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.word_end(self.caret());
        self.select_to(to, false);
        self.restart_blink(cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.ordered();
        if r.is_empty() {
            let start = self.word_start(r.start);
            self.replace(start..r.start, "", cx);
        } else {
            self.replace(r, "", cx);
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.ordered();
        if r.is_empty() {
            let start = self.prev_boundary(r.start);
            self.replace(start..r.start, "", cx);
        } else {
            self.replace(r, "", cx);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.ordered();
        if r.is_empty() {
            let end = self.next_boundary(r.end);
            self.replace(r.start..end, "", cx);
        } else {
            self.replace(r, "", cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.ordered();
        let to = if r.is_empty() { self.prev_boundary(r.start) } else { r.start };
        self.select_to(to, false);
        self.restart_blink(cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.ordered();
        let to = if r.is_empty() { self.next_boundary(r.end) } else { r.end };
        self.select_to(to, false);
        self.restart_blink(cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.prev_boundary(self.caret());
        self.select_to(to, true);
        self.restart_blink(cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.next_boundary(self.caret());
        self.select_to(to, true);
        self.restart_blink(cx);
    }

    fn vertical(&mut self, lines: f32, cx: &mut Context<Self>) {
        let Some((shaped, _)) = &self.layout else { return };
        let lh = self.line_height;
        let Some(pos) = shaped.visual_position_for_byte_index(self.caret(), lh) else { return };
        let target = point(pos.x, pos.y + lh * lines + lh / 2.);
        let to = if target.y < px(0.) {
            0
        } else {
            match shaped.closest_byte_index_for_pixel_point(target, lh) {
                Ok(i) | Err(i) => i,
            }
        };
        self.select_to(to, false);
        self.restart_blink(cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1., cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1., cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let line_start = self.text[..self.caret()].rfind('\n').map_or(0, |i| i + 1);
        self.select_to(line_start, false);
        self.restart_blink(cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let c = self.caret();
        let line_end = self.text[c..].find('\n').map_or(self.text.len(), |i| c + i);
        self.select_to(line_end, false);
        self.restart_blink(cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.anchor = 0;
        self.selection = 0..self.text.len();
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.ordered();
        if !r.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.text[r].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, w: &mut Window, cx: &mut Context<Self>) {
        let r = self.ordered();
        if !r.is_empty() {
            self.copy(&Copy, w, cx);
            self.replace(r, "", cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            let text = text.replace("\r\n", "\n");
            let r = self.ordered();
            self.replace(r, &text, cx);
        }
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.ordered();
        self.replace(r, "\n", cx);
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        let value = self.text.trim().to_string();
        if !value.is_empty() {
            cx.emit(Submitted(value));
        }
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        if let Some((shaped, bounds)) = &self.layout {
            let local = ev.position - bounds.origin;
            let ix = match shaped.closest_byte_index_for_pixel_point(local, self.line_height) {
                Ok(i) | Err(i) => i,
            };
            if ev.click_count == 2 && !self.text.is_empty() {
                let ix = ix.min(self.text.len());
                let (start, end) = (self.word_start(ix), self.word_end(ix));
                self.anchor = start;
                self.selection = start..end;
            } else {
                self.select_to(ix, ev.modifiers.shift);
            }
        }
        self.restart_blink(cx);
    }

    fn runs(&self, len: usize, color: gpui::Hsla) -> Vec<TextRun> {
        vec![TextRun {
            len,
            font: {
                let mut f = font(FONT_UI);
                f.features = ui_features();
                f
            },
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
            letter_spacing: None,
        }]
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = utf16_to_utf8_offset(&self.text, range.start)..utf16_to_utf8_offset(&self.text, range.end);
        actual.replace(utf8_to_utf16_offset(&self.text, r.start)..utf8_to_utf16_offset(&self.text, r.end));
        Some(self.text[r].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let r = self.ordered();
        Some(UTF16Selection {
            range: utf8_to_utf16_offset(&self.text, r.start)..utf8_to_utf16_offset(&self.text, r.end),
            reversed: self.selection.end < self.selection.start,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| {
            utf8_to_utf16_offset(&self.text, r.start)..utf8_to_utf16_offset(&self.text, r.end)
        })
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range
            .map(|r| utf16_to_utf8_offset(&self.text, r.start)..utf16_to_utf8_offset(&self.text, r.end))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.ordered());
        self.replace(r, new, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new: &str,
        _selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range
            .map(|r| utf16_to_utf8_offset(&self.text, r.start)..utf16_to_utf8_offset(&self.text, r.end))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.ordered());
        self.text.replace_range(r.clone(), new);
        self.marked = (!new.is_empty()).then(|| r.start..r.start + new.len());
        self.select_to(r.start + new.len(), false);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (shaped, bounds) = self.layout.as_ref()?;
        let start = utf16_to_utf8_offset(&self.text, range.start);
        let pos = shaped.visual_position_for_byte_index(start, self.line_height)?;
        Some(Bounds::new(bounds.origin + pos, size(px(2.), self.line_height)))
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let (shaped, bounds) = self.layout.as_ref()?;
        let ix = match shaped.closest_byte_index_for_pixel_point(p - bounds.origin, self.line_height) {
            Ok(i) | Err(i) => i,
        };
        Some(utf8_to_utf16_offset(&self.text, ix))
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("text-input")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor_text()
            .w_full()
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .child(InputText { input: cx.entity() })
    }
}

struct InputText {
    input: Entity<TextInput>,
}

struct Prepaint {
    shaped: Option<ShapedText>,
    caret: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
}

impl IntoElement for InputText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for InputText {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, ()) {
        let input = self.input.clone();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let id = window.request_measured_layout(style, move |known, available, window, cx| {
            let this = input.read(cx);
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let (text, color) = display_text(this, cx);
            let runs = this.runs(text.len(), color);
            let lines = window
                .text_system()
                .shape_text(text, this.font_size, &runs, width, None)
                .map(|s| s.line_count())
                .unwrap_or(1)
                .clamp(this.min_lines, this.max_lines);
            Size {
                width: width.unwrap_or(px(200.)),
                height: this.line_height * lines as f32,
            }
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let this = self.input.read(cx);
        let theme = cx.theme();
        let (text, color) = display_text(this, cx);
        let empty = this.text.is_empty();
        let runs = this.runs(text.len(), color);
        let lh = this.line_height;
        let shaped = window
            .text_system()
            .shape_text(text, this.font_size, &runs, Some(bounds.size.width), None)
            .expect("text shaping failed");
        let focused = this.focus_handle.is_focused(window);
        let range = this.ordered();
        let selection = if focused && !empty && !range.is_empty() {
            shaped
                .selection_bounds(range, lh)
                .into_iter()
                .map(|b| PaintQuad::new(Bounds::new(bounds.origin + b.origin, b.size)).background(theme.fg.opacity(0.18)))
                .collect()
        } else {
            Vec::new()
        };
        let caret = (focused && this.caret_visible).then(|| {
            let pos = if empty {
                point(px(0.), px(0.))
            } else {
                shaped
                    .visual_position_for_byte_index(this.caret(), lh)
                    .unwrap_or_default()
            };
            PaintQuad::new(Bounds::new(bounds.origin + pos, size(px(1.5), lh))).background(theme.fg)
        });
        Prepaint {
            shaped: Some(shaped),
            caret,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        for q in prepaint.selection.drain(..) {
            window.paint_quad(q);
        }
        let lh = self.input.read(cx).line_height;
        let Some(shaped) = prepaint.shaped.take() else { return };
        shaped
            .paint(bounds.origin, lh, TextAlign::Left, None, window, cx)
            .ok();
        if let Some(c) = prepaint.caret.take() {
            window.paint_quad(c);
        }
        // Keep the layout for hit-testing, caret movement and IME bounds.
        self.input.update(cx, |this, _| this.layout = Some((shaped, bounds)));
    }
}

fn display_text(this: &TextInput, cx: &App) -> (SharedString, gpui::Hsla) {
    let theme = cx.theme();
    if this.text.is_empty() {
        (this.placeholder.clone(), theme.fg_3)
    } else {
        (SharedString::from(this.text.clone()), theme.fg)
    }
}
