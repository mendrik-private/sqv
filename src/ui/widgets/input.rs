//! Single-line text entry with a char-based cursor and horizontal scrolling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use super::text::{char_width, put, sanitize};

fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

/// Inserts `ch` at char position `cursor` and advances the cursor.
pub fn insert_char(text: &mut String, cursor: &mut usize, ch: char) {
    text.insert(byte_index(text, *cursor), ch);
    *cursor += 1;
}

/// Deletes the char before the cursor; false when the cursor is at the start.
pub fn delete_backward(text: &mut String, cursor: &mut usize) -> bool {
    if *cursor == 0 {
        return false;
    }
    let start = byte_index(text, *cursor - 1);
    let end = byte_index(text, *cursor);
    text.replace_range(start..end, "");
    *cursor -= 1;
    true
}

/// A one-line editable value. Every popup query, filter needle and inline
/// insert field uses this, so they all share the same keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn set(&mut self, value: impl Into<String>) {
        *self = Self::new(value);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn insert(&mut self, ch: char) {
        insert_char(&mut self.value, &mut self.cursor, ch);
    }

    pub fn backspace(&mut self) -> bool {
        delete_backward(&mut self.value, &mut self.cursor)
    }

    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.value.chars().count() {
            return false;
        }
        self.cursor += 1;
        delete_backward(&mut self.value, &mut self.cursor)
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.value.chars().count());
    }

    fn delete_word_backward(&mut self) -> bool {
        let chars: Vec<char> = self.value.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        if start == self.cursor {
            return false;
        }
        let (from, to) = (
            byte_index(&self.value, start),
            byte_index(&self.value, self.cursor),
        );
        self.value.replace_range(from..to, "");
        self.cursor = start;
        true
    }

    /// Editing keys shared by every input. Returns true when the key was
    /// consumed; text-changing keys are reported through `changed`.
    pub fn handle_key(&mut self, key: &KeyEvent) -> InputOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let changed = match key.code {
            KeyCode::Char('a') if ctrl => {
                self.cursor = 0;
                false
            }
            KeyCode::Char('e') if ctrl => {
                self.cursor = self.value.chars().count();
                false
            }
            KeyCode::Char('u') if ctrl => {
                let end = byte_index(&self.value, self.cursor);
                self.value.replace_range(..end, "");
                self.cursor = 0;
                true
            }
            KeyCode::Char('w') if ctrl => self.delete_word_backward(),
            KeyCode::Char(ch) if !ctrl && !alt => {
                self.insert(ch);
                true
            }
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => {
                self.move_left();
                false
            }
            KeyCode::Right => {
                self.move_right();
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                false
            }
            KeyCode::End => {
                self.cursor = self.value.chars().count();
                false
            }
            _ => return InputOutcome::Ignored,
        };
        if changed {
            InputOutcome::Changed
        } else {
            InputOutcome::Moved
        }
    }

    /// Draws the value in one row, scrolled so the cursor stays visible, with
    /// ellipses marking text cut off on either side.
    pub fn render(
        &self,
        buf: &mut Buffer,
        area: Rect,
        style: Style,
        cursor: Option<(char, Style)>,
        ellipsis: char,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        buf.set_style(Rect { height: 1, ..area }, style);
        let chars: Vec<char> = sanitize(&self.value).chars().collect();
        let width = area.width as usize;
        let cursor_width = usize::from(cursor.is_some());
        let widths: Vec<usize> = chars.iter().map(|ch| char_width(*ch).max(1)).collect();
        let prefix = |to: usize| widths[..to].iter().sum::<usize>();

        // First visible char: as far left as possible while the cursor fits.
        let mut start = 0;
        while start < self.cursor
            && usize::from(start > 0) + prefix(self.cursor) - prefix(start) + cursor_width > width
        {
            start += 1;
        }
        // Last visible char, keeping a cell for the right ellipsis when cut.
        let left_marker = usize::from(start > 0);
        let mut end = start;
        let mut used = left_marker + cursor_width;
        while end < chars.len() {
            let right_marker = usize::from(end + 1 < chars.len());
            if used + widths[end] + right_marker > width {
                break;
            }
            used += widths[end];
            end += 1;
        }

        let right = area.right();
        let dim = style.add_modifier(Modifier::DIM);
        let mut x = area.x;
        if left_marker > 0 {
            x = put(buf, x, area.y, right, &ellipsis.to_string(), dim);
        }
        for (index, ch) in chars.iter().enumerate().take(end).skip(start) {
            if index == self.cursor {
                if let Some((glyph, cursor_style)) = cursor {
                    x = put(buf, x, area.y, right, &glyph.to_string(), cursor_style);
                }
            }
            x = put(buf, x, area.y, right, &ch.to_string(), style);
        }
        if self.cursor >= end {
            if let Some((glyph, cursor_style)) = cursor {
                x = put(buf, x, area.y, right, &glyph.to_string(), cursor_style);
            }
        }
        if end < chars.len() {
            put(buf, x, area.y, right, &ellipsis.to_string(), dim);
        }
    }
}

/// Draws `label` followed by the input on a raised field, as used by every
/// search box.
pub fn render_labeled(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    input: &TextInput,
    theme: &crate::theme::Theme,
    symbols: &crate::symbols::Symbols,
) {
    let label_style = Style::default().fg(theme.fg_dim).bg(theme.bg_raised);
    let x = put(buf, area.x, area.y, area.right(), label, label_style);
    let field = Rect::new(x, area.y, area.right().saturating_sub(x), 1);
    input.render(
        buf,
        field,
        Style::default().fg(theme.fg).bg(theme.bg_soft),
        Some((
            symbols.cursor,
            Style::default().fg(theme.accent).bg(theme.bg_soft),
        )),
        symbols.ellipsis,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOutcome {
    Ignored,
    Moved,
    Changed,
}

impl InputOutcome {
    pub fn consumed(self) -> bool {
        self != InputOutcome::Ignored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(input: &TextInput, width: u16) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 1));
        let area = buf.area;
        input.render(
            &mut buf,
            area,
            Style::default(),
            Some(('|', Style::default())),
            '…',
        );
        buf.content.iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn editing_keys_work_on_char_positions() {
        let mut input = TextInput::new("añb");
        input.handle_key(&KeyEvent::from(KeyCode::Left));
        input.handle_key(&KeyEvent::from(KeyCode::Char('é')));
        assert_eq!(input.value(), "añéb");
        input.handle_key(&KeyEvent::from(KeyCode::Delete));
        assert_eq!(input.value(), "añé");
        input.handle_key(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!((input.value(), input.cursor()), ("", 0));
    }

    #[test]
    fn long_values_scroll_to_keep_the_cursor_visible() {
        let input = TextInput::new("abcdefghij");
        assert_eq!(rendered(&input, 6), "…ghij|");
        let mut start = input.clone();
        start.handle_key(&KeyEvent::from(KeyCode::Home));
        assert_eq!(rendered(&start, 6), "|abcd…");
    }
}
