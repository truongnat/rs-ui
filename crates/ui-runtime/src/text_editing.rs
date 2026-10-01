//! Platform-neutral text editing state. Positions are grapheme-cluster indices.
use std::ops::Range;
use ui_core::{Point, TextRunId};
use ui_text::TextSystem;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextPosition(usize);
impl TextPosition {
    pub const fn new(grapheme_index: usize) -> Self {
        Self(grapheme_index)
    }
    pub const fn grapheme_index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextRange {
    pub start: TextPosition,
    pub end: TextPosition,
}
impl TextRange {
    pub fn new(a: TextPosition, b: TextPosition) -> Self {
        Self {
            start: a.min(b),
            end: a.max(b),
        }
    }
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: TextPosition,
    pub head: TextPosition,
}
impl Selection {
    pub const fn caret(at: TextPosition) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }
    pub fn range(self) -> TextRange {
        TextRange::new(self.anchor, self.head)
    }
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.head
    }
    pub fn collapse_left(&mut self) {
        let p = self.anchor.min(self.head);
        *self = Self::caret(p);
    }
    pub fn collapse_right(&mut self) {
        let p = self.anchor.max(self.head);
        *self = Self::caret(p);
    }
    pub fn extend_to(&mut self, head: TextPosition) {
        self.head = head;
    }
    /// Apply click selection after ui-text resolves the pointer to a logical position.
    pub fn click(&mut self, buffer: &TextBuffer, at: TextPosition, click_count: u8, extend: bool) {
        let at = buffer.clamp(at);
        let next = if click_count >= 3 {
            let line = buffer.line_range(buffer.line_index(at)).unwrap_or_default();
            Self {
                anchor: line.start,
                head: line.end,
            }
        } else if click_count == 2 {
            let (anchor, head) = buffer.word_range(at);
            Self { anchor, head }
        } else if extend {
            Self {
                anchor: at,
                head: at,
            }
        } else {
            Self::caret(at)
        };
        if extend {
            self.head = if next.is_collapsed() {
                at
            } else if at < self.anchor {
                next.anchor
            } else {
                next.head
            };
        } else {
            *self = next;
        }
    }
    pub fn drag_to(&mut self, head: TextPosition) {
        self.head = head;
    }
}

/// String-backed V1 buffer. Every public position is a grapheme boundary, never a byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextBuffer {
    text: String,
    grapheme_offsets: Vec<usize>,
}
impl TextBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let mut grapheme_offsets = text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        grapheme_offsets.push(text.len());
        Self {
            text,
            grapheme_offsets,
        }
    }
    pub fn text(&self) -> String {
        self.text.clone()
    }
    pub fn len(&self) -> usize {
        self.grapheme_offsets.len() - 1
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn line_count(&self) -> usize {
        self.text.split('\n').count()
    }
    pub fn line_index(&self, p: TextPosition) -> usize {
        self.text[..self.byte_offset(self.clamp(p))]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
    }
    pub fn clamp(&self, p: TextPosition) -> TextPosition {
        TextPosition(p.0.min(self.len()))
    }
    pub fn slice(&self, range: TextRange) -> String {
        let r = self.byte_range(range);
        self.text[r].to_owned()
    }
    pub fn line_range(&self, line: usize) -> Option<TextRange> {
        let mut start = 0;
        for (i, content) in self.text.split('\n').enumerate() {
            if i == line {
                return Some(TextRange::new(
                    TextPosition(start),
                    TextPosition(start + content.graphemes(true).count()),
                ));
            }
            start += content.graphemes(true).count() + 1;
        }
        None
    }
    pub fn insert(&mut self, at: TextPosition, text: &str) -> String {
        self.replace(TextRange::new(self.clamp(at), self.clamp(at)), text)
    }
    pub fn delete(&mut self, range: TextRange) -> String {
        self.replace(range, "")
    }
    pub fn replace(&mut self, range: TextRange, text: &str) -> String {
        let bytes = self.byte_range(range);
        let deleted = self.text[bytes.clone()].to_owned();
        self.replace_bytes(bytes, text);
        deleted
    }
    fn replace_bytes(&mut self, bytes: Range<usize>, text: &str) {
        self.text.replace_range(bytes, text);
        self.grapheme_offsets = self
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(self.text.len()))
            .collect();
    }
    fn position_at_or_after_byte(&self, byte: usize) -> TextPosition {
        TextPosition(
            self.grapheme_offsets
                .partition_point(|offset| *offset < byte.min(self.text.len())),
        )
    }
    pub fn previous(&self, p: TextPosition) -> TextPosition {
        TextPosition(self.clamp(p).0.saturating_sub(1))
    }
    pub fn next(&self, p: TextPosition) -> TextPosition {
        TextPosition((self.clamp(p).0 + 1).min(self.len()))
    }
    pub fn line_start(&self, p: TextPosition) -> TextPosition {
        let off = self.byte_offset(self.clamp(p));
        let start = self.text[..off].rfind('\n').map_or(0, |i| i + 1);
        TextPosition(self.text[..start].graphemes(true).count())
    }
    pub fn line_end(&self, p: TextPosition) -> TextPosition {
        let off = self.byte_offset(self.clamp(p));
        let end = self.text[off..]
            .find('\n')
            .map_or(self.text.len(), |i| off + i);
        TextPosition(self.text[..end].graphemes(true).count())
    }
    pub fn word_left(&self, p: TextPosition) -> TextPosition {
        let p = self.clamp(p);
        if p.0 == 0 {
            return p;
        }
        let byte = self.byte_offset(p);
        let left = &self.text[..byte];
        let mut target = 0;
        for (start, word) in left.unicode_word_indices() {
            if start + word.len() <= byte {
                target = start;
            }
        }
        if target == 0 {
            TextPosition(0)
        } else {
            TextPosition(self.text[..target].graphemes(true).count())
        }
    }
    pub fn word_right(&self, p: TextPosition) -> TextPosition {
        let p = self.clamp(p);
        let byte = self.byte_offset(p);
        if let Some((start, word)) = self.text[byte..].unicode_word_indices().next() {
            let end = byte + start + word.len();
            return TextPosition(self.text[..end].graphemes(true).count());
        }
        TextPosition(self.len())
    }
    pub fn word_range(&self, p: TextPosition) -> (TextPosition, TextPosition) {
        let p = self.clamp(p);
        let byte = self.byte_offset(p);
        for (start, word) in self.text.unicode_word_indices() {
            let end = start + word.len();
            if byte >= start && byte < end {
                return (
                    TextPosition(self.text[..start].graphemes(true).count()),
                    TextPosition(self.text[..end].graphemes(true).count()),
                );
            }
        }
        (p, self.next(p))
    }
    fn byte_range(&self, range: TextRange) -> Range<usize> {
        self.byte_offset(self.clamp(range.start))..self.byte_offset(self.clamp(range.end))
    }
    fn byte_offset(&self, p: TextPosition) -> usize {
        self.grapheme_offsets[p.0]
    }
}
impl Default for TextBuffer {
    fn default() -> Self {
        Self::new(String::new())
    }
}

pub trait Clipboard {
    fn get_text(&mut self) -> Option<String>;
    fn set_text(&mut self, text: &str);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditOperation {
    pub start: TextPosition,
    byte_start: usize,
    pub inserted: String,
    pub deleted: String,
    pub selection_before: Selection,
    pub selection_after: Selection,
}
#[derive(Clone, Debug, Default)]
pub struct UndoManager {
    undo: Vec<EditOperation>,
    redo: Vec<EditOperation>,
    group_open: bool,
}
impl UndoManager {
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }
    pub fn break_group(&mut self) {
        self.group_open = false;
    }
    fn record(&mut self, op: EditOperation, coalesce: bool) {
        self.redo.clear();
        if coalesce
            && self.group_open
            && op.deleted.is_empty()
            && op.selection_before.is_collapsed()
            && let Some(last) = self.undo.last_mut()
        {
            let expected = TextPosition(last.start.0 + last.inserted.graphemes(true).count());
            if last.deleted.is_empty()
                && op.start == expected
                && last.selection_after == op.selection_before
            {
                last.inserted.push_str(&op.inserted);
                last.selection_after = op.selection_after;
                return;
            }
        }
        self.undo.push(op);
        self.group_open = coalesce;
    }
    pub fn undo(&mut self, buffer: &mut TextBuffer, selection: &mut Selection) -> bool {
        self.break_group();
        let Some(op) = self.undo.pop() else {
            return false;
        };
        buffer.replace_bytes(
            op.byte_start..op.byte_start + op.inserted.len(),
            &op.deleted,
        );
        *selection = op.selection_before;
        self.redo.push(op);
        true
    }
    pub fn redo(&mut self, buffer: &mut TextBuffer, selection: &mut Selection) -> bool {
        self.break_group();
        let Some(op) = self.redo.pop() else {
            return false;
        };
        buffer.replace_bytes(
            op.byte_start..op.byte_start + op.deleted.len(),
            &op.inserted,
        );
        *selection = op.selection_after;
        self.undo.push(op);
        true
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImeState {
    pub composing_range: Option<TextRange>,
    pub preedit_text: String,
    pub cursor: Option<TextRange>,
}
impl ImeState {
    pub fn is_composing(&self) -> bool {
        self.composing_range.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditCommand {
    InsertText(String),
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    ExtendLeft,
    ExtendRight,
    ExtendWordLeft,
    ExtendWordRight,
    ExtendLineStart,
    ExtendLineEnd,
    ExtendDocumentStart,
    ExtendDocumentEnd,
    MoveWordLeft,
    MoveWordRight,
    MoveLineStart,
    MoveLineEnd,
    MoveDocumentStart,
    MoveDocumentEnd,
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
}

#[derive(Clone, Debug, Default)]
pub struct TextEditor {
    pub buffer: TextBuffer,
    pub selection: Selection,
    pub undo: UndoManager,
    pub ime: ImeState,
    preferred_x: Option<f32>,
}
impl TextEditor {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            buffer: TextBuffer::new(text),
            ..Self::default()
        }
    }
    pub fn execute(&mut self, command: EditCommand, clipboard: &mut impl Clipboard) {
        match command {
            EditCommand::InsertText(s) => self.replace_selection(&s, true),
            EditCommand::DeleteBackward
            | EditCommand::DeleteForward
            | EditCommand::DeleteWordBackward
            | EditCommand::DeleteWordForward => {
                let range = if !self.selection.is_collapsed() {
                    self.selection.range()
                } else if matches!(
                    command,
                    EditCommand::DeleteBackward | EditCommand::DeleteWordBackward
                ) {
                    let start = if matches!(command, EditCommand::DeleteWordBackward) {
                        self.buffer.word_left(self.selection.head)
                    } else {
                        self.buffer.previous(self.selection.head)
                    };
                    TextRange::new(start, self.selection.head)
                } else if matches!(command, EditCommand::DeleteWordForward) {
                    TextRange::new(
                        self.selection.head,
                        self.buffer.word_right(self.selection.head),
                    )
                } else {
                    TextRange::new(self.selection.head, self.buffer.next(self.selection.head))
                };
                self.replace_range(range, "", false);
            }
            EditCommand::MoveLeft | EditCommand::MoveRight => {
                if !self.selection.is_collapsed() {
                    if matches!(command, EditCommand::MoveLeft) {
                        self.selection.collapse_left();
                    } else {
                        self.selection.collapse_right();
                    }
                    self.undo.break_group();
                    self.ime_cancel();
                } else {
                    self.move_to(if matches!(command, EditCommand::MoveLeft) {
                        self.buffer.previous(self.selection.head)
                    } else {
                        self.buffer.next(self.selection.head)
                    });
                }
            }
            EditCommand::ExtendLeft | EditCommand::ExtendRight => {
                let head = if matches!(command, EditCommand::ExtendLeft) {
                    self.buffer.previous(self.selection.head)
                } else {
                    self.buffer.next(self.selection.head)
                };
                self.extend_to(head);
            }
            EditCommand::ExtendWordLeft => {
                self.extend_to(self.buffer.word_left(self.selection.head))
            }
            EditCommand::ExtendWordRight => {
                self.extend_to(self.buffer.word_right(self.selection.head))
            }
            EditCommand::ExtendLineStart => {
                self.extend_to(self.buffer.line_start(self.selection.head))
            }
            EditCommand::ExtendLineEnd => self.extend_to(self.buffer.line_end(self.selection.head)),
            EditCommand::ExtendDocumentStart => self.extend_to(TextPosition(0)),
            EditCommand::ExtendDocumentEnd => self.extend_to(TextPosition(self.buffer.len())),
            EditCommand::MoveWordLeft => {
                self.navigate(self.buffer.word_left(self.selection.head), false)
            }
            EditCommand::MoveWordRight => {
                self.navigate(self.buffer.word_right(self.selection.head), true)
            }
            EditCommand::MoveUp => self.move_vertical_logical(false),
            EditCommand::MoveDown => self.move_vertical_logical(true),
            EditCommand::MoveLineStart => {
                self.navigate(self.buffer.line_start(self.selection.head), false)
            }
            EditCommand::MoveLineEnd => {
                self.navigate(self.buffer.line_end(self.selection.head), true)
            }
            EditCommand::MoveDocumentStart => self.navigate(TextPosition(0), false),
            EditCommand::MoveDocumentEnd => self.navigate(TextPosition(self.buffer.len()), true),
            EditCommand::SelectAll => {
                self.undo.break_group();
                self.selection = Selection {
                    anchor: TextPosition(0),
                    head: TextPosition(self.buffer.len()),
                };
            }
            EditCommand::Copy => {
                self.undo.break_group();
                if !self.selection.is_collapsed() {
                    clipboard.set_text(&self.buffer.slice(self.selection.range()));
                }
            }
            EditCommand::Cut => {
                self.undo.break_group();
                if !self.selection.is_collapsed() {
                    clipboard.set_text(&self.buffer.slice(self.selection.range()));
                    self.replace_selection("", false);
                }
            }
            EditCommand::Paste => {
                self.undo.break_group();
                if let Some(text) = clipboard.get_text() {
                    self.replace_selection(&text.replace("\r\n", "\n").replace('\r', "\n"), false);
                }
            }
            EditCommand::Undo => {
                self.undo.undo(&mut self.buffer, &mut self.selection);
            }
            EditCommand::Redo => {
                self.undo.redo(&mut self.buffer, &mut self.selection);
            }
        }
    }
    pub fn replace_selection(&mut self, text: &str, typing: bool) {
        self.replace_range(self.selection.range(), text, typing);
    }
    pub fn execute_with_layout(
        &mut self,
        command: EditCommand,
        clipboard: &mut impl Clipboard,
        text: &TextSystem,
        run: TextRunId,
        extend: bool,
    ) {
        match command {
            EditCommand::MoveUp => {
                self.move_vertical(text, run, false, extend);
            }
            EditCommand::MoveDown => {
                self.move_vertical(text, run, true, extend);
            }
            EditCommand::MoveLineStart | EditCommand::ExtendLineStart => {
                self.move_visual_line_boundary(
                    text,
                    run,
                    false,
                    matches!(command, EditCommand::ExtendLineStart),
                );
            }
            EditCommand::MoveLineEnd | EditCommand::ExtendLineEnd => {
                self.move_visual_line_boundary(
                    text,
                    run,
                    true,
                    matches!(command, EditCommand::ExtendLineEnd),
                );
            }
            other => self.execute(other, clipboard),
        }
    }
    pub fn move_visual_line_boundary(
        &mut self,
        text: &TextSystem,
        run: TextRunId,
        end: bool,
        extend: bool,
    ) -> bool {
        let position = self.selection.head.grapheme_index();
        let Some(lines) = text.line_metrics(run) else {
            return false;
        };
        let line = if end {
            lines
                .iter()
                .find(|line| position >= line.start && position <= line.end)
        } else {
            lines
                .iter()
                .rev()
                .find(|line| position >= line.start && position <= line.end)
        };
        let Some(line) = line else {
            return false;
        };
        let target =
            TextPosition::new(if end { line.end } else { line.start }.min(self.buffer.len()));
        self.undo.break_group();
        self.ime_cancel();
        self.preferred_x = None;
        if extend {
            self.selection.extend_to(target);
        } else {
            self.selection = Selection::caret(target);
        }
        true
    }
    pub fn move_vertical(
        &mut self,
        text: &TextSystem,
        run: TextRunId,
        down: bool,
        extend: bool,
    ) -> bool {
        if self.ime.is_composing() {
            return false;
        }
        let Some(point) = text.position_to_point(run, self.selection.head.grapheme_index()) else {
            return false;
        };
        let Some(caret) = text.caret_rect(run, self.selection.head.grapheme_index(), 0.0) else {
            return false;
        };
        let x = *self.preferred_x.get_or_insert(point.x);
        let target = Point::new(
            x,
            point.y
                + if down {
                    caret.height()
                } else {
                    -caret.height()
                },
        );
        let Some(position) = text.point_to_position(run, target) else {
            return false;
        };
        let head = TextPosition::new(position.min(self.buffer.len()));
        self.undo.break_group();
        self.ime_cancel();
        if extend {
            self.selection.extend_to(head);
        } else {
            self.selection = Selection::caret(head);
        }
        true
    }
    pub fn set_selection(&mut self, selection: Selection) {
        self.undo.break_group();
        self.preferred_x = None;
        self.selection = Selection {
            anchor: self.buffer.clamp(selection.anchor),
            head: self.buffer.clamp(selection.head),
        };
    }
    fn extend_to(&mut self, target: TextPosition) {
        self.undo.break_group();
        self.selection.extend_to(self.buffer.clamp(target));
        self.ime_cancel();
        self.preferred_x = None;
    }
    pub fn pointer_click(&mut self, at: TextPosition, click_count: u8, extend: bool) {
        self.undo.break_group();
        self.preferred_x = None;
        self.selection.click(&self.buffer, at, click_count, extend);
    }
    pub fn pointer_drag_to(&mut self, head: TextPosition) {
        self.undo.break_group();
        self.preferred_x = None;
        self.selection.drag_to(self.buffer.clamp(head));
    }
    fn move_vertical_logical(&mut self, down: bool) {
        let line = self.buffer.line_index(self.selection.head);
        let Some(current) = self.buffer.line_range(line) else {
            return;
        };
        let target_line = if down {
            line + 1
        } else {
            line.saturating_sub(1)
        };
        let Some(target) = self.buffer.line_range(target_line) else {
            return;
        };
        let column = self.selection.head.0.saturating_sub(current.start.0);
        self.navigate(
            TextPosition((target.start.0 + column).min(target.end.0)),
            down,
        );
    }
    /// Materialize the temporary IME presentation without changing committed buffer/history.
    pub fn presentation_text(&self) -> String {
        let Some(range) = self.ime.composing_range else {
            return self.buffer.text();
        };
        let mut text = self.buffer.text();
        let bytes = self.buffer.byte_range(range);
        text.replace_range(bytes, &self.ime.preedit_text);
        text
    }
    fn replace_range(&mut self, range: TextRange, text: &str, typing: bool) {
        let before = self.selection;
        let range = TextRange::new(self.buffer.clamp(range.start), self.buffer.clamp(range.end));
        if range.is_empty() && text.is_empty() {
            self.undo.break_group();
            return;
        }
        let byte_start = self.buffer.byte_offset(range.start);
        let deleted = self.buffer.slice(range);
        self.buffer.replace(range, text);
        let head = self
            .buffer
            .position_at_or_after_byte(byte_start + text.len());
        self.selection = Selection::caret(head);
        self.undo.record(
            EditOperation {
                start: range.start,
                byte_start,
                inserted: text.to_owned(),
                deleted,
                selection_before: before,
                selection_after: self.selection,
            },
            typing,
        );
        self.ime = ImeState::default();
        self.preferred_x = None;
    }
    fn move_to(&mut self, p: TextPosition) {
        self.undo.break_group();
        self.selection = Selection::caret(self.buffer.clamp(p));
        self.ime = ImeState::default();
        self.preferred_x = None;
    }
    fn navigate(&mut self, target: TextPosition, rightward: bool) {
        if !self.selection.is_collapsed() {
            if rightward {
                self.selection.collapse_right();
            } else {
                self.selection.collapse_left();
            }
            self.undo.break_group();
            self.ime_cancel();
        } else {
            self.move_to(target);
        }
    }
    pub fn ime_start(&mut self) {
        self.undo.break_group();
        self.ime.composing_range = Some(self.selection.range());
        self.ime.preedit_text.clear();
        self.ime.cursor = None;
    }
    pub fn ime_preedit(&mut self, text: &str, cursor: Option<TextRange>) {
        if !self.ime.is_composing() {
            self.ime_start();
        }
        self.ime.preedit_text = text.to_owned();
        self.ime.cursor = cursor;
    }
    pub fn ime_commit(&mut self, text: &str) {
        let range = self
            .ime
            .composing_range
            .take()
            .unwrap_or_else(|| self.selection.range());
        self.ime = ImeState::default();
        self.replace_range(range, text, false);
        self.undo.break_group();
    }
    pub fn ime_cancel(&mut self) {
        self.ime = ImeState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct MemClipboard(Option<String>);
    impl Clipboard for MemClipboard {
        fn get_text(&mut self) -> Option<String> {
            self.0.clone()
        }
        fn set_text(&mut self, text: &str) {
            self.0 = Some(text.to_owned());
        }
    }
    #[test]
    fn grapheme_boundaries_cover_unicode_and_delete_as_units() {
        for s in [
            "Xin chào Việt Nam",
            "Trường",
            "ắ",
            "e\u{301}",
            "👨‍👩‍👧‍👦",
            "🇻🇳",
            "日本語",
            "한국어",
        ] {
            let mut b = TextBuffer::new(s);
            assert_eq!(b.len(), 1.max(s.graphemes(true).count()));
            let expected = b.text();
            let mut cursor = TextPosition::new(0);
            while cursor < TextPosition::new(b.len()) {
                let next = b.next(cursor);
                assert_eq!(b.previous(next), cursor);
                let grapheme = b.slice(TextRange::new(cursor, next));
                assert!(!grapheme.is_empty());
                let deleted = b.delete(TextRange::new(cursor, next));
                assert_eq!(deleted, grapheme);
                b.insert(cursor, &deleted);
                cursor = next;
            }
            let first = TextRange::new(TextPosition::new(0), TextPosition::new(1));
            let first_grapheme = b.slice(first);
            b.replace(first, "x");
            assert_eq!(b.slice(first), "x");
            b.replace(first, &first_grapheme);
            assert_eq!(b.text(), expected);
            while !b.is_empty() {
                let p = b.previous(TextPosition(b.len()));
                b.delete(TextRange::new(p, TextPosition(b.len())));
            }
            assert!(b.is_empty());
        }
    }
    #[test]
    fn directional_selection_replace_and_lines() {
        let mut e = TextEditor::new("one\ntwo");
        e.selection = Selection {
            anchor: TextPosition(7),
            head: TextPosition(4),
        };
        assert_eq!(e.buffer.slice(e.selection.range()), "two");
        e.replace_selection("三", false);
        assert_eq!(e.buffer.text(), "one\n三");
        assert_eq!(e.buffer.line_count(), 2);
        assert_eq!(
            e.buffer.line_range(1).unwrap(),
            TextRange::new(TextPosition(4), TextPosition(5))
        );
    }
    #[test]
    fn pointer_selection_keeps_direction_and_supports_word_line_and_drag() {
        let b = TextBuffer::new("xin chào\nViệt Nam");
        let mut s = Selection::caret(TextPosition(0));
        s.click(&b, TextPosition(5), 2, false);
        assert_eq!(b.slice(s.range()), "chào");
        s.click(&b, TextPosition(10), 3, false);
        assert_eq!(b.slice(s.range()), "Việt Nam");
        s.click(&b, TextPosition(3), 1, false);
        s.drag_to(TextPosition(1));
        assert_eq!(s.anchor, TextPosition(3));
        assert_eq!(s.head, TextPosition(1));
    }
    #[test]
    fn typing_coalesces_and_redo_branch_is_cleared() {
        let mut e = TextEditor::default();
        let mut c = MemClipboard::default();
        for ch in "hello".chars() {
            e.execute(EditCommand::InsertText(ch.to_string()), &mut c);
        }
        assert_eq!(e.undo.undo_depth(), 1);
        e.execute(EditCommand::Undo, &mut c);
        assert_eq!(e.buffer.text(), "");
        e.execute(EditCommand::Redo, &mut c);
        assert_eq!(e.buffer.text(), "hello");
        e.execute(EditCommand::MoveLeft, &mut c);
        e.execute(EditCommand::InsertText("!".into()), &mut c);
        assert_eq!(e.undo.redo_depth(), 0);
    }
    #[test]
    fn non_editing_commands_break_typing_groups_and_boundary_deletes_are_noops() {
        let mut editor = TextEditor::default();
        let mut clipboard = MemClipboard::default();
        editor.execute(EditCommand::InsertText("hi".into()), &mut clipboard);
        editor.execute(EditCommand::Copy, &mut clipboard);
        editor.execute(EditCommand::InsertText("!".into()), &mut clipboard);
        assert_eq!(editor.undo.undo_depth(), 2);
        editor.execute(EditCommand::Undo, &mut clipboard);
        assert_eq!(editor.buffer.text(), "hi");
        editor.set_selection(Selection::caret(TextPosition::new(0)));
        editor.execute(EditCommand::DeleteBackward, &mut clipboard);
        assert_eq!(editor.undo.undo_depth(), 1);
        assert_eq!(editor.buffer.text(), "hi");
    }
    #[test]
    fn delete_and_directional_selection_replacement_undo_redo_exactly() {
        let mut editor = TextEditor::new("a🇻🇳b");
        let mut clipboard = MemClipboard::default();
        editor.set_selection(Selection::caret(TextPosition::new(2)));
        editor.execute(EditCommand::DeleteBackward, &mut clipboard);
        assert_eq!(editor.buffer.text(), "ab");
        editor.execute(EditCommand::Undo, &mut clipboard);
        assert_eq!(editor.buffer.text(), "a🇻🇳b");
        editor.set_selection(Selection {
            anchor: TextPosition::new(3),
            head: TextPosition::new(1),
        });
        editor.replace_selection("候補", false);
        assert_eq!(editor.buffer.text(), "a候補");
        editor.execute(EditCommand::Undo, &mut clipboard);
        assert_eq!(editor.buffer.text(), "a🇻🇳b");
        assert_eq!(
            editor.selection,
            Selection {
                anchor: TextPosition::new(3),
                head: TextPosition::new(1)
            }
        );
        editor.execute(EditCommand::Redo, &mut clipboard);
        assert_eq!(editor.buffer.text(), "a候補");
    }
    #[test]
    fn combining_and_zwj_edits_remain_boundary_safe_and_byte_history_is_reversible() {
        let mut editor = TextEditor::new("e");
        let mut clipboard = MemClipboard::default();
        editor.set_selection(Selection::caret(TextPosition::new(1)));
        editor.execute(EditCommand::InsertText("\u{301}".into()), &mut clipboard);
        assert_eq!(editor.buffer.text(), "e\u{301}");
        assert_eq!(editor.buffer.len(), 1);
        assert_eq!(editor.selection.head, TextPosition::new(1));
        editor.execute(EditCommand::Undo, &mut clipboard);
        assert_eq!(editor.buffer.text(), "e");
        editor.execute(EditCommand::Redo, &mut clipboard);
        assert_eq!(editor.buffer.text(), "e\u{301}");

        let mut editor = TextEditor::new("👨👩");
        editor.set_selection(Selection::caret(TextPosition::new(1)));
        editor.execute(EditCommand::InsertText("\u{200D}".into()), &mut clipboard);
        assert_eq!(editor.buffer.text(), "👨‍👩");
        assert_eq!(editor.buffer.len(), 1);
        editor.execute(EditCommand::Undo, &mut clipboard);
        assert_eq!(editor.buffer.text(), "👨👩");
        editor.execute(EditCommand::Redo, &mut clipboard);
        assert_eq!(editor.buffer.text(), "👨‍👩");
    }
    #[test]
    fn horizontal_commands_collapse_or_extend_directional_selections() {
        let mut e = TextEditor::new("ắ🇻🇳x");
        let mut c = MemClipboard::default();
        e.selection = Selection {
            anchor: TextPosition(0),
            head: TextPosition(2),
        };
        e.execute(EditCommand::MoveLeft, &mut c);
        assert_eq!(e.selection, Selection::caret(TextPosition(0)));
        e.execute(EditCommand::ExtendRight, &mut c);
        assert_eq!(
            e.selection,
            Selection {
                anchor: TextPosition(0),
                head: TextPosition(1)
            }
        );
        e.execute(EditCommand::DeleteBackward, &mut c);
        assert_eq!(e.buffer.text(), "🇻🇳x");
    }
    #[test]
    fn vertical_layout_navigation_preserves_preferred_x_across_short_lines() {
        let mut text = TextSystem::new();
        let run = text.shape(
            "abcdefghij\nab\nabcdefghij",
            ui_text::TextStyle::default(),
            None,
        );
        let mut editor = TextEditor::new("abcdefghij\nab\nabcdefghij");
        editor.set_selection(Selection::caret(TextPosition::new(8)));
        assert!(editor.move_vertical(&text, run, true, false));
        let middle = editor.buffer.line_range(1).unwrap();
        assert_eq!(editor.selection.head, middle.end);
        assert!(editor.move_vertical(&text, run, true, false));
        assert_eq!(editor.selection.head, TextPosition::new(8 + 10 + 1 + 2 + 1));
    }
    #[test]
    fn home_end_resolve_visual_wrapped_line_boundaries_from_shaped_text() {
        let mut text = TextSystem::new();
        let value = "abcdefghijklmno";
        let run = text.shape(value, ui_text::TextStyle::default(), Some(25.0));
        let visual_lines = text.line_metrics(run).unwrap();
        assert!(visual_lines.len() > 1);
        let line = visual_lines[1];
        let mut editor = TextEditor::new(value);
        editor.set_selection(Selection::caret(TextPosition::new(line.start + 1)));
        assert!(editor.move_visual_line_boundary(&text, run, false, false));
        assert_eq!(editor.selection.head, TextPosition::new(line.start));
        editor.set_selection(Selection::caret(TextPosition::new(line.start + 1)));
        assert!(editor.move_visual_line_boundary(&text, run, true, false));
        assert_eq!(editor.selection.head, TextPosition::new(line.end));
    }
    #[test]
    fn clipboard_paste_cut_and_ime_commit_are_single_undo_operations() {
        let mut e = TextEditor::new("abc");
        let mut c = MemClipboard(Some("x\r\ny".into()));
        e.selection = Selection {
            anchor: TextPosition(1),
            head: TextPosition(2),
        };
        e.execute(EditCommand::Cut, &mut c);
        assert_eq!(c.0.as_deref(), Some("b"));
        c.0 = Some("x\r\ny".into());
        e.execute(EditCommand::Paste, &mut c);
        assert_eq!(e.buffer.text(), "ax\nyc");
        e.ime_start();
        e.ime_preedit("候", Some(TextRange::default()));
        assert_eq!(e.buffer.text(), "ax\nyc");
        assert_eq!(e.presentation_text(), "ax\ny候c");
        e.ime_commit("候");
        e.execute(EditCommand::Undo, &mut c);
        assert_eq!(e.buffer.text(), "ax\nyc");
    }
    #[test]
    fn ime_cancel_discards_preedit_without_touching_buffer_selection_or_history() {
        let mut editor = TextEditor::new("abc");
        let selection = Selection {
            anchor: TextPosition::new(3),
            head: TextPosition::new(1),
        };
        editor.set_selection(selection);
        let depth = editor.undo.undo_depth();
        editor.ime_start();
        editor.ime_preedit("k", Some(TextRange::default()));
        editor.ime_preedit("仮", Some(TextRange::default()));
        assert_eq!(editor.presentation_text(), "a仮");
        editor.ime_cancel();
        assert_eq!(editor.presentation_text(), "abc");
        assert_eq!(editor.selection, selection);
        assert_eq!(editor.undo.undo_depth(), depth);
    }
    #[test]
    fn unicode_word_boundaries_and_line_boundaries() {
        let b = TextBuffer::new("Xin chào Việt Nam\n日本語");
        assert!(b.word_right(TextPosition(0)) > TextPosition(0));
        let vietnamese_word = b.word_range(TextPosition::new(11));
        assert_eq!(
            b.slice(TextRange::new(vietnamese_word.0, vietnamese_word.1)),
            "Việt"
        );
        assert_eq!(b.word_left(TextPosition::new(13)), TextPosition::new(9));
        assert_eq!(b.word_right(TextPosition::new(9)), TextPosition::new(13));
        let first_line = b.line_range(0).unwrap();
        let second_start = TextPosition(first_line.end.grapheme_index() + 1);
        assert_eq!(b.line_start(second_start), second_start);
        assert_eq!(b.line_end(first_line.end), first_line.end);
    }
}
