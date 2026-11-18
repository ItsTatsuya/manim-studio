//! A soft-wrapping text input used by the composers.
//!
//! Text lives in the entity; one [`ShapedText`] covers the whole value, so caret
//! and selection geometry come straight from GPUI's text layout. The editable
//! state (text, selection, IME mark, undo history) lives in [`Buffer`], which has
//! no GPUI dependencies so its rules can be unit tested without a window.

use crate::theme::{ActiveTheme, FONT_UI, text, ui_features};
use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, ColorExt, ContentMask, Context, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels,
    Point, Render, ShapedText, SharedString, Size, Style, Styled, Task, TextAlign, TextRun,
    UTF16Selection, Window, actions, div, font, point, px, relative, size, utf8_to_utf16_offset,
    utf16_to_utf8_offset,
};
use std::cell::RefCell;
use std::ops::Range;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_input,
    [
        Backspace, Delete, Left, Right, Up, Down, Home, End, SelectLeft, SelectRight, SelectAll,
        SelectHome, SelectEnd, Copy, Cut, Paste, Submit, Newline, WordLeft, WordRight,
        DeleteWordLeft, Undo, Redo,
    ]
);

const CONTEXT: &str = "TextInput";
/// Undo snapshots kept per input.
const HISTORY_LIMIT: usize = 100;
/// Width of the painted caret, plus a little slack so it is never clipped at the edge.
const CARET_WIDTH: f32 = 1.5;

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
        KeyBinding::new("shift-home", SelectHome, ctx),
        KeyBinding::new("shift-end", SelectEnd, ctx),
        KeyBinding::new(&format!("{secondary}-a"), SelectAll, ctx),
        KeyBinding::new(&format!("{secondary}-c"), Copy, ctx),
        KeyBinding::new(&format!("{secondary}-x"), Cut, ctx),
        KeyBinding::new(&format!("{secondary}-v"), Paste, ctx),
        KeyBinding::new(&format!("{secondary}-z"), Undo, ctx),
        KeyBinding::new(&format!("{secondary}-shift-z"), Redo, ctx),
        KeyBinding::new(&format!("{secondary}-y"), Redo, ctx),
        KeyBinding::new(&format!("{word}-left"), WordLeft, ctx),
        KeyBinding::new(&format!("{word}-right"), WordRight, ctx),
        KeyBinding::new(&format!("{word}-backspace"), DeleteWordLeft, ctx),
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new("shift-enter", Newline, ctx),
    ]);
}

/// Emitted when the user presses Enter with non-empty text (or with any text
/// after [`TextInput::submit_empty`]).
pub struct Submitted(pub String);

/// Emitted when the input loses focus (inline renames commit on it).
pub struct Blurred;

// ---------------------------------------------------------------------------
// Pure helpers (no GPUI types, unit tested below)
// ---------------------------------------------------------------------------

/// Clamp `offset` to `text` and move it down to the nearest char boundary.
pub(crate) fn snap(text: &str, offset: usize) -> usize {
    let mut o = offset.min(text.len());
    while !text.is_char_boundary(o) {
        o -= 1;
    }
    o
}

/// Clamp both ends to `text`, snap them to char boundaries and order them.
pub(crate) fn snap_range(text: &str, r: Range<usize>) -> Range<usize> {
    let (a, b) = (snap(text, r.start), snap(text, r.end));
    a.min(b)..a.max(b)
}

/// Single-line inputs turn every line break (`\r\n`, `\n`, `\r`) into one space.
pub(crate) fn strip_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\n' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// New scroll offset (one axis) that keeps `[start, start + len)` inside the
/// viewport, never past the content and never negative.
pub(crate) fn follow(scroll: f32, start: f32, len: f32, view: f32, content: f32) -> f32 {
    let max = (content - view).max(0.);
    let mut s = scroll;
    if start < s {
        s = start;
    } else if start + len > s + view {
        s = start + len - view;
    }
    s.clamp(0., max)
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    text: String,
    selection: Range<usize>,
    anchor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    /// One non-space character typed at the caret.
    Typing,
    /// One character removed.
    Deleting,
    /// Anything else (paste, cut, composition, replace a selection, ...).
    Other,
}

/// Bounded undo/redo of whole-text snapshots. Consecutive typing (or consecutive
/// single-character deletes) share one snapshot.
#[derive(Default)]
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    run: Option<EditKind>,
}

impl History {
    /// Call before an edit. `snap` captures the pre-edit state; it is only invoked
    /// when a new undo step is needed.
    fn record(&mut self, kind: EditKind, snap: impl FnOnce() -> Snapshot) {
        if kind != EditKind::Other && self.run == Some(kind) {
            return;
        }
        self.run = Some(kind);
        self.redo.clear();
        self.undo.push(snap());
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
    }

    /// Cursor movement ends a typing run: the next edit starts a new step.
    fn break_run(&mut self) {
        self.run = None;
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.undo.pop()?;
        self.redo.push(current);
        self.run = None;
        Some(prev)
    }

    fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        self.run = None;
        Some(next)
    }
}

/// The editable state. Every offset stored here is a char boundary within `text`:
/// all writes go through [`snap`]/[`snap_range`].
struct Buffer {
    text: String,
    /// Byte range; `selection.end` is the caret.
    selection: Range<usize>,
    anchor: usize,
    /// IME composition range (bytes).
    marked: Option<Range<usize>>,
    single_line: bool,
    /// Bumped on every text change; keys the shared-string cache.
    rev: u64,
    history: History,
}

impl Buffer {
    fn new() -> Self {
        Self {
            text: String::new(),
            selection: 0..0,
            anchor: 0,
            marked: None,
            single_line: false,
            rev: 0,
            history: History::default(),
        }
    }

    fn clean(&self, s: &str) -> String {
        if self.single_line { strip_newlines(s) } else { s.to_string() }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), selection: self.selection.clone(), anchor: self.anchor }
    }

    fn restore(&mut self, s: Snapshot) {
        let reversed = s.selection.end < s.selection.start;
        self.text = s.text;
        self.marked = None;
        self.rev += 1;
        let r = snap_range(&self.text, s.selection);
        self.selection = if reversed { r.end..r.start } else { r };
        self.anchor = snap(&self.text, s.anchor);
    }

    fn caret(&self) -> usize {
        self.selection.end
    }

    fn ordered(&self) -> Range<usize> {
        let (a, b) = (self.selection.start, self.selection.end);
        a.min(b)..a.max(b)
    }

    /// Replace the whole value; drops the IME mark and the undo history.
    fn set_text(&mut self, text: &str, select_all: bool) {
        self.text = self.clean(text);
        self.marked = None;
        self.rev += 1;
        self.history.clear();
        let end = self.text.len();
        if select_all {
            self.anchor = 0;
            self.selection = 0..end;
        } else {
            self.anchor = end;
            self.selection = end..end;
        }
    }

    fn select_to(&mut self, offset: usize, extend: bool) {
        let offset = snap(&self.text, offset);
        if extend {
            self.anchor = snap(&self.text, self.anchor);
        } else {
            self.anchor = offset;
        }
        self.selection = self.anchor..offset;
    }

    fn select_all(&mut self) {
        self.anchor = 0;
        self.selection = 0..self.text.len();
    }

    /// Range an IME / platform call acts on: the given UTF-8 range, else the
    /// composition, else the selection; always valid for `text`.
    fn resolve(&self, range: Option<Range<usize>>) -> Range<usize> {
        let r = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.ordered());
        snap_range(&self.text, r)
    }

    /// Replace `range` and put the caret after the new text.
    fn replace(&mut self, range: Range<usize>, new: &str) {
        let range = snap_range(&self.text, range);
        let new = self.clean(new);
        if range.is_empty() && new.is_empty() {
            return;
        }
        if self.marked.is_none() {
            let kind = self.classify(&range, &new);
            let snap = self.snapshot();
            self.history.record(kind, || snap);
        }
        self.text.replace_range(range.clone(), &new);
        self.rev += 1;
        self.marked = None;
        self.select_to(range.start + new.len(), false);
    }

    fn classify(&self, range: &Range<usize>, new: &str) -> EditKind {
        let mut chars = new.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if range.is_empty() && !c.is_whitespace() => EditKind::Typing,
            (None, _) if self.text[range.clone()].chars().count() == 1 => EditKind::Deleting,
            _ => EditKind::Other,
        }
    }

    /// IME composition: replace and mark the inserted text.
    fn replace_marking(&mut self, range: Option<Range<usize>>, new: &str) {
        let r = self.resolve(range);
        let new = self.clean(new);
        if self.marked.is_none() {
            let snap = self.snapshot();
            self.history.record(EditKind::Other, || snap);
        }
        self.text.replace_range(r.clone(), &new);
        self.rev += 1;
        self.select_to(r.start + new.len(), false);
        self.marked = (!new.is_empty()).then(|| r.start..r.start + new.len());
    }

    fn undo(&mut self) -> bool {
        let cur = self.snapshot();
        match self.history.undo(cur) {
            Some(s) => {
                self.restore(s);
                true
            }
            None => false,
        }
    }

    fn redo(&mut self) -> bool {
        let cur = self.snapshot();
        match self.history.redo(cur) {
            Some(s) => {
                self.restore(s);
                true
            }
            None => false,
        }
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
        let before = &self.text[..snap(&self.text, offset)];
        let trimmed = before.trim_end();
        trimmed
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8())
    }

    fn word_end(&self, offset: usize) -> usize {
        let offset = snap(&self.text, offset);
        let after = &self.text[offset..];
        let skip = after.len() - after.trim_start().len();
        let rest = &after[skip..];
        offset + skip + rest.find(char::is_whitespace).unwrap_or(rest.len())
    }

    fn line_start(&self, offset: usize) -> usize {
        self.text[..snap(&self.text, offset)].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self, offset: usize) -> usize {
        let c = snap(&self.text, offset);
        self.text[c..].find('\n').map_or(self.text.len(), |i| c + i)
    }
}

// ---------------------------------------------------------------------------

pub struct TextInput {
    focus_handle: FocusHandle,
    buf: Buffer,
    placeholder: SharedString,
    caret_visible: bool,
    blink: Task<()>,
    font_size: Pixels,
    line_height: Pixels,
    min_lines: usize,
    max_lines: usize,
    submit_empty: bool,
    /// Scroll offset of the text inside the box (y for multi-line, x for single-line).
    scroll: Point<Pixels>,
    /// True while the left button is held after a press inside the input.
    selecting: bool,
    /// Layout of the last paint. Cleared whenever the text changes, so it never
    /// describes a different string than `buf.text`.
    layout: Option<(ShapedText, Bounds<Pixels>)>,
    /// Cached `SharedString` of the text, keyed by `Buffer::rev`.
    shared: RefCell<Option<(u64, SharedString)>>,
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
            this.selecting = false;
            cx.emit(Blurred);
            cx.notify();
        })
        .detach();
        Self {
            focus_handle,
            buf: Buffer::new(),
            placeholder: placeholder.into(),
            caret_visible: false,
            blink: Task::ready(()),
            font_size: text::BODY,
            line_height: px(20.),
            min_lines: 1,
            max_lines: 8,
            submit_empty: false,
            scroll: Point::default(),
            selecting: false,
            layout: None,
            shared: RefCell::new(None),
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.buf.set_text(&text, false);
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
        self.buf.single_line = self.max_lines == 1;
        // Re-clean text set before the line mode was known.
        if self.buf.single_line {
            let t = self.buf.text.clone();
            self.buf.set_text(&t, false);
        }
        self
    }

    /// Also emit [`Submitted`] when Enter is pressed on empty (or blank) text.
    pub fn submit_empty(mut self, yes: bool) -> Self {
        self.submit_empty = yes;
        self
    }

    pub fn text(&self) -> &str {
        &self.buf.text
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let text = text.into();
        self.buf.set_text(&text, false);
        self.text_replaced();
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    /// Replace the text and select all of it (ready to type over, as in a rename).
    pub fn set_text_selected(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let text = text.into();
        self.buf.set_text(&text, true);
        self.text_replaced();
        cx.notify();
    }

    /// Wholesale replacement: the old layout and scroll no longer apply.
    fn text_replaced(&mut self) {
        self.layout = None;
        self.scroll = Point::default();
        self.selecting = false;
    }

    fn single_line(&self) -> bool {
        self.buf.single_line
    }

    /// The text as a `SharedString`, cloned only when it changed.
    fn shared_text(&self) -> SharedString {
        let mut cache = self.shared.borrow_mut();
        match &*cache {
            Some((rev, s)) if *rev == self.buf.rev => s.clone(),
            _ => {
                let s = SharedString::from(self.buf.text.clone());
                *cache = Some((self.buf.rev, s.clone()));
                s
            }
        }
    }

    fn select_to(&mut self, offset: usize, extend: bool) {
        self.buf.select_to(offset, extend);
    }

    /// Cursor movement: ends the undo typing run and restarts the blink.
    fn moved(&mut self, cx: &mut Context<Self>) {
        self.buf.history.break_run();
        self.restart_blink(cx);
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
        self.buf.replace(range, new);
        self.layout = None;
        self.restart_blink(cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buf.word_start(self.buf.caret());
        self.select_to(to, false);
        self.moved(cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buf.word_end(self.buf.caret());
        self.select_to(to, false);
        self.moved(cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.buf.ordered();
        if r.is_empty() {
            let start = self.buf.word_start(r.start);
            self.replace(start..r.start, "", cx);
        } else {
            self.replace(r, "", cx);
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.buf.ordered();
        if r.is_empty() {
            let start = self.buf.prev_boundary(r.start);
            self.replace(start..r.start, "", cx);
        } else {
            self.replace(r, "", cx);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.buf.ordered();
        if r.is_empty() {
            let end = self.buf.next_boundary(r.end);
            self.replace(r.start..end, "", cx);
        } else {
            self.replace(r, "", cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.buf.ordered();
        let to = if r.is_empty() { self.buf.prev_boundary(r.start) } else { r.start };
        self.select_to(to, false);
        self.moved(cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.buf.ordered();
        let to = if r.is_empty() { self.buf.next_boundary(r.end) } else { r.end };
        self.select_to(to, false);
        self.moved(cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buf.prev_boundary(self.buf.caret());
        self.select_to(to, true);
        self.moved(cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buf.next_boundary(self.buf.caret());
        self.select_to(to, true);
        self.moved(cx);
    }

    fn vertical(&mut self, lines: f32, cx: &mut Context<Self>) {
        let to = match &self.layout {
            Some((shaped, _)) => {
                let lh = self.line_height;
                let Some(pos) = shaped.visual_position_for_byte_index(self.buf.caret(), lh) else {
                    return;
                };
                let target = point(pos.x, pos.y + lh * lines + lh / 2.);
                if target.y < px(0.) {
                    0
                } else {
                    match shaped.closest_byte_index_for_pixel_point(target, lh) {
                        Ok(i) | Err(i) => i,
                    }
                }
            }
            // No fresh layout yet (an edit since the last paint): stay put rather
            // than jumping to the start or end; the next key press has a layout.
            None => return,
        };
        self.select_to(to, false);
        self.moved(cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1., cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1., cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buf.line_start(self.buf.caret());
        self.select_to(to, false);
        self.moved(cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buf.line_end(self.buf.caret());
        self.select_to(to, false);
        self.moved(cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buf.line_start(self.buf.caret());
        self.select_to(to, true);
        self.moved(cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.buf.line_end(self.buf.caret());
        self.select_to(to, true);
        self.moved(cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.buf.select_all();
        self.buf.history.break_run();
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.buf.ordered();
        if !r.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.text[r].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, w: &mut Window, cx: &mut Context<Self>) {
        let r = self.buf.ordered();
        if !r.is_empty() {
            self.copy(&Copy, w, cx);
            self.replace(r, "", cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            // Single-line inputs turn newlines into spaces inside `Buffer::replace`.
            let text = text.replace("\r\n", "\n");
            let r = self.buf.ordered();
            self.replace(r, &text, cx);
        }
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if self.single_line() {
            return;
        }
        let r = self.buf.ordered();
        self.replace(r, "\n", cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.buf.undo() {
            self.layout = None;
            self.restart_blink(cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.buf.redo() {
            self.layout = None;
            self.restart_blink(cx);
        }
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        let value = self.buf.text.trim().to_string();
        if !value.is_empty() || self.submit_empty {
            cx.emit(Submitted(value));
        }
    }

    /// Byte offset under a window position, from the last paint's layout.
    fn hit(&self, pos: Point<Pixels>) -> Option<usize> {
        let (shaped, bounds) = self.layout.as_ref()?;
        let local = pos - bounds.origin + self.scroll;
        let ix = match shaped.closest_byte_index_for_pixel_point(local, self.line_height) {
            Ok(i) | Err(i) => i,
        };
        Some(snap(&self.buf.text, ix))
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        // Without a fresh layout (just edited) keep the selection and only focus.
        if let Some(ix) = self.hit(ev.position) {
            if ev.click_count == 2 && !self.buf.text.is_empty() {
                let (start, end) = (self.buf.word_start(ix), self.buf.word_end(ix));
                self.buf.anchor = start;
                self.buf.selection = start..end;
            } else {
                self.select_to(ix, ev.modifiers.shift);
            }
            self.selecting = true;
        }
        self.moved(cx);
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !(self.selecting && ev.dragging()) {
            return;
        }
        if let Some(ix) = self.hit(ev.position) {
            self.select_to(ix, true);
            self.moved(cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
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

    /// Wrap width for shaping: single-line inputs never wrap (they scroll).
    fn wrap_width(&self, available: Option<Pixels>) -> Option<Pixels> {
        if self.single_line() { None } else { available }
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
        let t = &self.buf.text;
        let r = snap_range(t, utf16_to_utf8_offset(t, range.start)..utf16_to_utf8_offset(t, range.end));
        actual.replace(utf8_to_utf16_offset(t, r.start)..utf8_to_utf16_offset(t, r.end));
        Some(t[r].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let r = self.buf.ordered();
        let t = &self.buf.text;
        Some(UTF16Selection {
            range: utf8_to_utf16_offset(t, r.start)..utf8_to_utf16_offset(t, r.end),
            reversed: self.buf.selection.end < self.buf.selection.start,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let t = &self.buf.text;
        self.buf.marked.as_ref().map(|r| {
            let r = snap_range(t, r.clone());
            utf8_to_utf16_offset(t, r.start)..utf8_to_utf16_offset(t, r.end)
        })
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.buf.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let t = &self.buf.text;
        let utf8 = range.map(|r| utf16_to_utf8_offset(t, r.start)..utf16_to_utf8_offset(t, r.end));
        let r = self.buf.resolve(utf8);
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
        let t = &self.buf.text;
        let utf8 = range.map(|r| utf16_to_utf8_offset(t, r.start)..utf16_to_utf8_offset(t, r.end));
        self.buf.replace_marking(utf8, new);
        self.layout = None;
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
        let start = snap(&self.buf.text, utf16_to_utf8_offset(&self.buf.text, range.start));
        let pos = shaped.visual_position_for_byte_index(start, self.line_height)?;
        Some(Bounds::new(bounds.origin + pos - self.scroll, size(px(2.), self.line_height)))
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let ix = self.hit(p)?;
        Some(utf8_to_utf16_offset(&self.buf.text, ix))
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
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
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
    scroll: Point<Pixels>,
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
                .shape_text(text, this.font_size, &runs, this.wrap_width(width), None)
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
        let empty = this.buf.text.is_empty();
        let runs = this.runs(text.len(), color);
        let lh = this.line_height;
        let single = this.single_line();
        let shaped = window
            .text_system()
            .shape_text(text, this.font_size, &runs, this.wrap_width(Some(bounds.size.width)), None)
            .expect("text shaping failed");
        let focused = this.focus_handle.is_focused(window);
        let range = this.buf.ordered();
        let caret_pos = if empty {
            point(px(0.), px(0.))
        } else {
            shaped
                .visual_position_for_byte_index(this.buf.caret(), lh)
                .unwrap_or_default()
        };
        // Follow the caret: horizontally for single-line, vertically otherwise.
        let scroll = if empty {
            Point::default()
        } else {
            let content = shaped.size(lh);
            if single {
                point(
                    px(follow(
                        f32::from(this.scroll.x),
                        f32::from(caret_pos.x),
                        CARET_WIDTH + 0.5,
                        f32::from(bounds.size.width),
                        f32::from(content.width) + CARET_WIDTH + 0.5,
                    )),
                    px(0.),
                )
            } else {
                point(
                    px(0.),
                    px(follow(
                        f32::from(this.scroll.y),
                        f32::from(caret_pos.y),
                        f32::from(lh),
                        f32::from(bounds.size.height),
                        f32::from(content.height),
                    )),
                )
            }
        };
        let origin = bounds.origin - scroll;
        let selection = if focused && !empty && !range.is_empty() {
            shaped
                .selection_bounds(range, lh)
                .into_iter()
                .map(|b| PaintQuad::new(Bounds::new(origin + b.origin, b.size)).background(theme.fg.opacity(0.18)))
                .collect()
        } else {
            Vec::new()
        };
        let caret = (focused && this.caret_visible).then(|| {
            PaintQuad::new(Bounds::new(origin + caret_pos, size(px(CARET_WIDTH), lh))).background(theme.fg)
        });
        Prepaint {
            shaped: Some(shaped),
            caret,
            selection,
            scroll,
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
        let lh = self.input.read(cx).line_height;
        let Some(shaped) = prepaint.shaped.take() else { return };
        let scroll = prepaint.scroll;
        let selection: Vec<PaintQuad> = std::mem::take(&mut prepaint.selection);
        let caret = prepaint.caret.take();
        // Text taller/wider than the box is clipped to it.
        let mask = ContentMask { bounds, fade_out: Default::default() };
        window.with_content_mask(Some(mask), |window| {
            for q in selection {
                window.paint_quad(q);
            }
            shaped
                .paint(bounds.origin - scroll, lh, TextAlign::Left, None, window, cx)
                .ok();
            if let Some(c) = caret {
                window.paint_quad(c);
            }
        });
        // Keep the layout for hit-testing, caret movement and IME bounds.
        self.input.update(cx, |this, _| {
            this.layout = Some((shaped, bounds));
            this.scroll = scroll;
        });
    }
}

fn display_text(this: &TextInput, cx: &App) -> (SharedString, gpui::Hsla) {
    let theme = cx.theme();
    if this.buf.text.is_empty() {
        (this.placeholder.clone(), theme.fg_3)
    } else {
        (this.shared_text(), theme.fg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        let mut b = Buffer::new();
        b.set_text(text, false);
        b
    }

    fn typed(b: &mut Buffer, s: &str) {
        for c in s.chars() {
            let at = b.caret();
            b.replace(at..at, &c.to_string());
        }
    }

    #[test]
    fn stale_ranges_are_clamped_and_snapped() {
        let mut b = buf("aé€");
        // Platform range past the end and in the middle of chars.
        let r = b.resolve(Some(2..99));
        assert_eq!(r, 1..6);
        b.replace(r, "Z");
        assert_eq!(b.text, "aZ");
        assert_eq!(b.caret(), 2);
        // Stale composition range after the text got shorter.
        b.marked = Some(10..20);
        assert_eq!(b.resolve(None), 2..2);
        b.replace_marking(None, "é");
        assert_eq!(b.text, "aZé");
        assert_eq!(b.marked, Some(2..4));
    }

    #[test]
    fn word_and_line_helpers_accept_mid_char_offsets() {
        let b = buf("héllo wörld\nnext");
        let _ = b.word_start(2);
        let _ = b.word_end(2);
        assert_eq!(b.line_start(3), 0);
        assert_eq!(b.line_end(3), 13);
    }

    #[test]
    fn typing_coalesces_into_one_undo_step() {
        let mut b = Buffer::new();
        typed(&mut b, "abc");
        assert!(b.undo());
        assert_eq!(b.text, "");
        assert!(!b.undo());
        assert!(b.redo());
        assert_eq!(b.text, "abc");
        assert_eq!(b.caret(), 3);
        assert!(!b.redo());
    }

    #[test]
    fn deletes_coalesce_and_restore_selection() {
        let mut b = buf("hello");
        b.replace(4..5, "");
        b.replace(3..4, "");
        assert_eq!(b.text, "hel");
        assert!(b.undo());
        assert_eq!(b.text, "hello");
        assert!(!b.undo(), "both deletes were one step");
        assert_eq!(b.selection, 5..5);
    }

    #[test]
    fn composition_commit_is_one_undo_step() {
        let mut b = buf("x");
        b.replace_marking(None, "か");
        b.replace_marking(None, "かな");
        let r = b.resolve(None);
        b.replace(r, "仮名");
        assert_eq!(b.marked, None);
        assert_eq!(b.text, "x仮名");
        assert!(b.undo());
        assert_eq!(b.text, "x");
    }

    #[test]
    fn follow_keeps_caret_in_view() {
        // Caret below the viewport scrolls down just enough.
        assert_eq!(follow(0., 90., 20., 100., 300.), 10.);
        // Caret above scrolls up.
        assert_eq!(follow(50., 20., 20., 100., 300.), 20.);
        // Already visible: unchanged.
        assert_eq!(follow(10., 40., 20., 100., 300.), 10.);
        // Never past the content, never negative.
        assert_eq!(follow(500., 0., 20., 100., 90.), 0.);
        assert_eq!(follow(0., 290., 20., 100., 300.), 200.);
    }
}
