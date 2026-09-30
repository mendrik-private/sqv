//! Selection and scrolling for vertical lists.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};

use super::text::put;
use crate::{symbols::Symbols, theme::Theme};

/// The selected item of a list plus a stored scroll offset. The offset only
/// moves when the selection would leave the viewport, so the list stays still
/// while the selection travels inside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListCursor {
    pub selected: usize,
    offset: usize,
    /// Rows visible at the last render; paging moves by this much.
    viewport: usize,
}

impl ListCursor {
    pub fn reset(&mut self) {
        self.selected = 0;
        self.offset = 0;
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn select(&mut self, index: usize, len: usize) {
        self.selected = index.min(len.saturating_sub(1));
        self.follow(len);
    }

    pub fn move_by(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        let target = self.selected as isize + delta;
        self.selected = target.clamp(0, len as isize - 1) as usize;
        self.follow(len);
    }

    /// Keeps the offset on the selection between renders, so clicks right
    /// after a key hit what the next frame shows.
    fn follow(&mut self, len: usize) {
        if self.viewport > 0 {
            self.visible(len, self.viewport);
        }
    }

    fn page(&self) -> isize {
        self.viewport.saturating_sub(1).max(1) as isize
    }

    /// Up/Down, PageUp/PageDown and Home/End; true when the key was a list key.
    pub fn handle_key(&mut self, key: &KeyEvent, len: usize) -> bool {
        match key.code {
            KeyCode::Up => self.move_by(-1, len),
            KeyCode::Down => self.move_by(1, len),
            KeyCode::PageUp => self.move_by(-self.page(), len),
            KeyCode::PageDown => self.move_by(self.page(), len),
            KeyCode::Home => self.select(0, len),
            KeyCode::End => self.select(len.saturating_sub(1), len),
            _ => return false,
        }
        true
    }

    /// Wheel scrolling moves the selection by three rows.
    pub fn scroll(&mut self, down: bool, len: usize) {
        self.move_by(if down { 3 } else { -3 }, len);
    }

    /// Adjusts the offset so the selection is visible in `viewport` rows and
    /// returns the index range to draw.
    pub fn visible(&mut self, len: usize, viewport: usize) -> std::ops::Range<usize> {
        self.viewport = viewport;
        self.selected = self.selected.min(len.saturating_sub(1));
        if viewport == 0 || len == 0 {
            self.offset = 0;
            return 0..0;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + viewport {
            self.offset = self.selected + 1 - viewport;
        }
        self.offset = self.offset.min(len.saturating_sub(viewport));
        self.offset..(self.offset + viewport).min(len)
    }

    /// The item under row `row_in_view` of the last render, if any.
    pub fn hit(&self, row_in_view: usize, len: usize) -> Option<usize> {
        let index = self.offset + row_in_view;
        (row_in_view < self.viewport && index < len).then_some(index)
    }
}

/// Background for a list row: selected rows are raised, others sit on the popup.
pub fn row_style(theme: &Theme, selected: bool) -> Style {
    if selected {
        Style::default().bg(theme.bg_soft).fg(theme.fg)
    } else {
        Style::default().bg(theme.bg_raised).fg(theme.fg_dim)
    }
}

/// Fills a list row and draws the selection marker in its first cell.
pub fn paint_row(buf: &mut Buffer, row: Rect, selected: bool, theme: &Theme, symbols: &Symbols) {
    let style = row_style(theme, selected);
    buf.set_style(row, style);
    if selected {
        put(
            buf,
            row.x,
            row.y,
            row.right(),
            &symbols.selection.to_string(),
            style.fg(theme.accent),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_moves_only_when_the_selection_leaves_the_viewport() {
        let mut cursor = ListCursor::default();
        cursor.visible(20, 5);
        cursor.move_by(7, 20);
        assert_eq!(cursor.visible(20, 5), 3..8);
        cursor.move_by(-1, 20);
        assert_eq!(
            cursor.visible(20, 5),
            3..8,
            "moving up inside the view keeps it"
        );
        cursor.move_by(-4, 20);
        assert_eq!(cursor.visible(20, 5), 2..7);
    }

    #[test]
    fn paging_uses_the_rendered_viewport() {
        let mut cursor = ListCursor::default();
        cursor.visible(100, 10);
        cursor.handle_key(&KeyEvent::from(KeyCode::PageDown), 100);
        assert_eq!(cursor.selected, 9);
        cursor.handle_key(&KeyEvent::from(KeyCode::End), 100);
        assert_eq!(cursor.selected, 99);
        assert_eq!(cursor.hit(9, 100), Some(99));
    }
}
