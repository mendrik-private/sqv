//! A read-only result table: a header, typed and highlighted cells, a pinned
//! first column, horizontal column scrolling and a selectable row list.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Modifier, Style},
};

use super::{
    cell::{cell_text, fit_cell, Align},
    highlighted_spans,
    list::{paint_row, ListCursor},
    scrollbar::Scrollbar,
    state::{render_state, StateView},
    text::{put, sanitize, text_width, truncate_with_ellipsis},
};
use crate::{
    db::types::{ColumnKind, SqlValue},
    symbols::Symbols,
    theme::Theme,
};

const MAX_COLUMN_WIDTH: usize = 40;
const MIN_COLUMN_WIDTH: usize = 4;

#[derive(Debug, Default)]
pub struct ResultTable {
    headers: Vec<String>,
    kinds: Vec<ColumnKind>,
    rows: Vec<Vec<SqlValue>>,
    /// Formatted text per cell, computed once when the rows arrive.
    display: Vec<Vec<(String, Align)>>,
    widths: Vec<usize>,
    /// Lowercased needle whose occurrences are highlighted.
    highlight: String,
    pub list: ListCursor,
    /// First scrollable column; column 0 stays pinned on the left.
    h_offset: usize,
    body: Rect,
    hidden_left: bool,
    hidden_right: bool,
}

impl ResultTable {
    pub fn set_data(
        &mut self,
        headers: Vec<String>,
        kinds: Vec<ColumnKind>,
        rows: Vec<Vec<SqlValue>>,
        symbols: &Symbols,
    ) {
        self.display = rows
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(col, value)| {
                        let kind = kinds.get(col).copied().unwrap_or(ColumnKind::Untyped);
                        let (text, align) = cell_text(value, kind, symbols);
                        (text.into_owned(), align)
                    })
                    .collect()
            })
            .collect();
        self.widths = headers
            .iter()
            .enumerate()
            .map(|(col, header)| {
                let content = self
                    .display
                    .iter()
                    .filter_map(|row| row.get(col))
                    .map(|(text, _)| text_width(text))
                    .max()
                    .unwrap_or(0);
                content
                    .max(text_width(header))
                    .clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH)
            })
            .collect();
        self.headers = headers.iter().map(|h| sanitize(h).into_owned()).collect();
        self.kinds = kinds;
        self.rows = rows;
        self.list.reset();
        self.h_offset = 0;
    }

    pub fn set_highlight(&mut self, needle: &str) {
        self.highlight = needle.to_lowercase();
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn selected(&self) -> Option<usize> {
        (self.list.selected < self.rows.len()).then_some(self.list.selected)
    }

    pub fn selected_row(&self) -> Option<&[SqlValue]> {
        self.rows.get(self.selected()?).map(Vec::as_slice)
    }

    fn scroll_columns(&mut self, right: bool) {
        let last = self.headers.len().saturating_sub(1);
        if right && self.hidden_right {
            self.h_offset = (self.h_offset.max(1) + 1).min(last);
        } else if !right {
            self.h_offset = self.h_offset.saturating_sub(1).max(1);
            if self.h_offset == 1 {
                self.h_offset = 0;
            }
        }
    }

    /// List keys plus Ctrl-←/→ for columns. Returns true when consumed.
    pub fn handle_key(&mut self, key: &KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Left => {
                    self.scroll_columns(false);
                    return true;
                }
                KeyCode::Right => {
                    self.scroll_columns(true);
                    return true;
                }
                _ => {}
            }
        }
        self.list.handle_key(key, self.rows.len())
    }

    /// Wheel scrolls rows, Shift-wheel scrolls columns, a click selects. A click
    /// on the already selected row returns `true` to confirm it.
    pub fn handle_mouse(
        &mut self,
        kind: MouseEventKind,
        modifiers: KeyModifiers,
        x: u16,
        y: u16,
    ) -> Option<bool> {
        match kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                if modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.scroll_columns(kind == MouseEventKind::ScrollDown);
                Some(false)
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                self.scroll_columns(kind == MouseEventKind::ScrollRight);
                Some(false)
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                self.list
                    .scroll(kind == MouseEventKind::ScrollDown, self.rows.len());
                Some(false)
            }
            MouseEventKind::Down(MouseButton::Left) if self.body.contains(Position { x, y }) => {
                let index = self.list.hit((y - self.body.y) as usize, self.rows.len())?;
                if index == self.list.selected {
                    return Some(true);
                }
                self.list.select(index, self.rows.len());
                Some(false)
            }
            _ => None,
        }
    }

    /// Columns shown for the current horizontal offset, with their widths.
    fn layout(&mut self, width: usize) -> Vec<(usize, usize)> {
        let mut columns = Vec::new();
        if self.headers.is_empty() {
            return columns;
        }
        let mut used = 0;
        let order = std::iter::once(0).chain(self.h_offset.max(1)..self.headers.len());
        for col in order {
            let separator = usize::from(!columns.is_empty());
            let wanted = self.widths[col];
            let available = width.saturating_sub(used + separator);
            if available < MIN_COLUMN_WIDTH.min(wanted) {
                break;
            }
            let shown = wanted.min(available);
            columns.push((col, shown));
            used += separator + shown;
            if shown < wanted {
                break;
            }
        }
        self.hidden_left = self.h_offset > 1;
        self.hidden_right = columns
            .last()
            .is_some_and(|(col, _)| col + 1 < self.headers.len())
            || columns.last().is_some_and(|&(col, w)| w < self.widths[col]);
        // Give leftover width to the last column so rows fill the table.
        let total =
            columns.iter().map(|(_, width)| width).sum::<usize>() + columns.len().saturating_sub(1);
        if let Some(last) = columns
            .last_mut()
            .filter(|_| total < width && !self.hidden_right)
        {
            last.1 += width - total;
        }
        columns
    }

    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        empty: StateView<'_>,
        theme: &Theme,
        symbols: &Symbols,
    ) {
        let bg = theme.bg_raised;
        if area.height < 3 || area.width < 4 {
            return;
        }
        // Two-cell gutter for the selection marker and one for the scrollbar.
        let content = Rect::new(area.x + 2, area.y, area.width - 3, area.height);
        let columns = self.layout(content.width as usize);
        let header_style = Style::default()
            .fg(theme.fg)
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        let rule_style = Style::default().fg(theme.line).bg(bg);
        let mut x = content.x;
        for (index, &(col, width)) in columns.iter().enumerate() {
            if index > 0 {
                put(
                    buf,
                    x,
                    area.y,
                    content.right(),
                    &symbols.box_vertical.to_string(),
                    rule_style,
                );
                x += 1;
            }
            let header = truncate_with_ellipsis(&self.headers[col], width, symbols.ellipsis);
            put(buf, x, area.y, x + width as u16, &header, header_style);
            x += width as u16;
        }
        if self.hidden_left {
            put(
                buf,
                area.x,
                area.y,
                area.x + 2,
                "‹",
                rule_style.fg(theme.accent),
            );
        }
        if self.hidden_right {
            put(
                buf,
                area.right() - 1,
                area.y,
                area.right(),
                "›",
                rule_style.fg(theme.accent),
            );
        }
        let rule: String = symbols
            .box_horizontal
            .to_string()
            .repeat(area.width as usize);
        put(buf, area.x, area.y + 1, area.right(), &rule, rule_style);

        let body = Rect::new(
            area.x,
            area.y + 2,
            area.width.saturating_sub(1),
            area.height - 2,
        );
        self.body = body;
        if self.rows.is_empty() {
            render_state(buf, body, empty, bg, theme, symbols);
            return;
        }
        let range = self.list.visible(self.rows.len(), body.height as usize);
        for (row_in_view, index) in range.enumerate() {
            let y = body.y + row_in_view as u16;
            let selected = index == self.list.selected;
            paint_row(
                buf,
                Rect::new(body.x, y, body.width, 1),
                selected,
                theme,
                symbols,
            );
            let row_bg = if selected { theme.bg_soft } else { bg };
            let base = Style::default()
                .fg(if selected { theme.fg } else { theme.fg_dim })
                .bg(row_bg);
            let matched_style = base.fg(theme.accent).add_modifier(Modifier::BOLD);
            let mut x = content.x;
            for (position, &(col, width)) in columns.iter().enumerate() {
                if position > 0 {
                    put(
                        buf,
                        x,
                        y,
                        content.right(),
                        &symbols.box_vertical.to_string(),
                        rule_style.bg(row_bg),
                    );
                    x += 1;
                }
                let (text, align) = self.display[index]
                    .get(col)
                    .map(|(text, align)| (text.as_str(), *align))
                    .unwrap_or(("", Align::Left));
                let cell = Rect::new(x, y, width as u16, 1);
                self.render_cell(buf, cell, text, align, (base, matched_style), symbols);
                x += width as u16;
            }
        }
        Scrollbar {
            offset: self.list.offset(),
            total: self.rows.len(),
            viewport: body.height as usize,
        }
        .render(
            buf,
            Rect::new(area.right() - 1, body.y, 1, body.height),
            bg,
            theme,
            symbols,
        );
    }

    /// Draws one cell; `styles` are the plain and the highlighted style.
    fn render_cell(
        &self,
        buf: &mut Buffer,
        cell: Rect,
        text: &str,
        align: Align,
        (base, matched): (Style, Style),
        symbols: &Symbols,
    ) {
        let (x, y, width) = (cell.x, cell.y, cell.width as usize);
        let range = (!self.highlight.is_empty())
            .then(|| {
                let lower = text.to_lowercase();
                let start = lower.find(&self.highlight)?;
                let start = lower[..start].chars().count();
                Some((start, start + self.highlight.chars().count()))
            })
            .flatten();
        if range.is_none() && align != Align::Left {
            let fitted = fit_cell(text, width, align, symbols.ellipsis);
            let pad = width.saturating_sub(text_width(&fitted));
            let offset = if align == Align::Right { pad } else { pad / 2 };
            put(buf, x + offset as u16, y, x + width as u16, &fitted, base);
            return;
        }
        let is_matched = |idx: usize| range.is_some_and(|(start, end)| idx >= start && idx < end);
        let mut cx = x;
        for span in highlighted_spans(text, is_matched, width, base, matched) {
            cx = put(buf, cx, y, x + width as u16, &span.content, span.style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(columns: usize, rows: usize) -> ResultTable {
        let symbols = Symbols::default_with_nerd_font(false);
        let mut table = ResultTable::default();
        table.set_data(
            (0..columns).map(|c| format!("column_{c}")).collect(),
            vec![ColumnKind::Text; columns],
            (0..rows)
                .map(|r| {
                    (0..columns)
                        .map(|c| SqlValue::Text(format!("r{r}c{c}")))
                        .collect()
                })
                .collect(),
            &symbols,
        );
        table
    }

    fn render_text(table: &mut ResultTable, width: u16, height: u16) -> Vec<String> {
        let theme = Theme::default();
        let symbols = Symbols::default_with_nerd_font(false);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        let area = buf.area;
        table.render(
            &mut buf,
            area,
            StateView::Empty("none", None),
            &theme,
            &symbols,
        );
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn wide_tables_scroll_columns_and_mark_hidden_ones() {
        let mut wide = table(6, 3);
        let first = render_text(&mut wide, 40, 5);
        assert!(first[0].contains("column_0") && first[0].ends_with('›'));
        wide.handle_key(&KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        wide.handle_key(&KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        let scrolled = render_text(&mut wide, 40, 5);
        assert!(scrolled[0].starts_with('‹'));
        assert!(
            scrolled[0].contains("column_0"),
            "the first column stays pinned"
        );
        assert!(scrolled[0].contains("column_3"));
    }

    #[test]
    fn selection_follows_into_view() {
        let mut long = table(2, 50);
        long.handle_key(&KeyEvent::from(KeyCode::End));
        let lines = render_text(&mut long, 30, 6);
        assert!(lines[5].contains("r49c0"));
    }
}
