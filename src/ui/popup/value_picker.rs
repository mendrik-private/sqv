use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEventKind};
use ratatui::{
    layout::{Position, Rect},
    style::{Modifier, Style},
    Frame,
};

use super::PopupAction;
use crate::{
    db::types::{expects_number, parse_input, SqlValue},
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        fuzzy_filter, highlighted_spans,
        hints::{hint, render_hints},
        input::{render_labeled, TextInput},
        list::{paint_row, ListCursor},
        state::{render_state, StateView},
        text::{put, sanitize},
    },
};

/// Picks one of a column's existing values, or a new value typed into the
/// search field. Matches come first; the typed value is the last entry.
pub struct ValuePickerState {
    pub table: String,
    pub rowid: i64,
    pub col_name: String,
    pub col_type: String,
    pub values: Vec<String>,
    pub query: TextInput,
    pub list: ListCursor,
    pub original: SqlValue,
    list_area: Rect,
}

enum Entry<'a> {
    Existing {
        raw: &'a str,
        display: String,
        matched: Vec<usize>,
    },
    Typed(&'a str),
}

impl ValuePickerState {
    pub fn new(
        table: String,
        rowid: i64,
        col_name: String,
        col_type: String,
        values: Vec<String>,
        original: SqlValue,
    ) -> Self {
        Self {
            table,
            rowid,
            col_name,
            col_type,
            values,
            query: TextInput::default(),
            list: ListCursor::default(),
            original,
            list_area: Rect::default(),
        }
    }

    fn entries(&self) -> Vec<Entry<'_>> {
        picker_entries(&self.values, &self.query, &self.col_type)
    }

    pub fn selected_value(&self) -> Option<String> {
        self.entries()
            .get(self.list.selected)
            .map(|entry| match entry {
                Entry::Existing { raw, .. } => raw.to_string(),
                Entry::Typed(value) => value.to_string(),
            })
    }

    pub fn selected_sql_value(&self) -> Option<SqlValue> {
        parse_input(&self.col_type, &self.selected_value()?).ok()
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        match key.code {
            KeyCode::Esc => return PopupAction::Close,
            KeyCode::Enter => return PopupAction::Submit,
            KeyCode::Home | KeyCode::End if !self.query.is_empty() => {}
            _ if self.list.handle_key(key, self.entries().len()) => return PopupAction::Handled,
            _ => {}
        }
        if self.query.handle_key(key).consumed() {
            self.list.reset();
            PopupAction::Handled
        } else {
            PopupAction::Ignored
        }
    }

    pub fn handle_mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> PopupAction {
        let len = self.entries().len();
        match kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                self.list.scroll(kind == MouseEventKind::ScrollDown, len);
                PopupAction::Handled
            }
            MouseEventKind::Down(MouseButton::Left)
                if self.list_area.contains(Position { x, y }) =>
            {
                match self.list.hit((y - self.list_area.y) as usize, len) {
                    Some(index) if index == self.list.selected => PopupAction::Submit,
                    Some(index) => {
                        self.list.select(index, len);
                        PopupAction::Handled
                    }
                    None => PopupAction::Handled,
                }
            }
            _ => PopupAction::Ignored,
        }
    }

    fn accepts_new_values(&self) -> bool {
        !expects_number(&self.col_type) || !self.values.is_empty()
    }
}

fn picker_entries<'a>(
    values: &'a [String],
    query: &'a TextInput,
    col_type: &str,
) -> Vec<Entry<'a>> {
    let mut entries = fuzzy_filter(
        values
            .iter()
            .map(|value| (value.as_str(), sanitize(value).into_owned())),
        query.value(),
    )
    .into_iter()
    .map(|(raw, display, matched)| Entry::Existing {
        raw,
        display,
        matched,
    })
    .collect::<Vec<_>>();
    let typed = query.value().trim();
    let is_existing = values.iter().any(|value| value == typed);
    if !typed.is_empty() && !is_existing && parse_input(col_type, typed).is_ok() {
        entries.push(Entry::Typed(query.value()));
    }
    entries
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut ValuePickerState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let inner = PopupFrame::new(
        "Pick value",
        Some(&state.col_name),
        (area.width / 2).max(36),
        16,
    )
    .render(frame, area, theme);
    if inner.height < 4 {
        return;
    }
    let buf = frame.buffer_mut();
    let bg = theme.bg_raised;
    render_labeled(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        " Search or type: ",
        &state.query,
        theme,
        symbols,
    );

    let list_area = Rect::new(inner.x, inner.y + 2, inner.width, inner.height - 3);
    state.list_area = list_area;
    let entries = picker_entries(&state.values, &state.query, &state.col_type);
    if entries.is_empty() {
        let message = if state.accepts_new_values() {
            "No match; type a valid value to add it"
        } else {
            "No match"
        };
        render_state(
            buf,
            list_area,
            StateView::Empty(message, None),
            bg,
            theme,
            symbols,
        );
    }
    for (row, index) in state
        .list
        .visible(entries.len(), list_area.height as usize)
        .enumerate()
    {
        let selected = index == state.list.selected;
        let row_rect = Rect::new(list_area.x, list_area.y + row as u16, list_area.width, 1);
        paint_row(buf, row_rect, selected, theme, symbols);
        let row_bg = if selected { theme.bg_soft } else { bg };
        let base = Style::default()
            .fg(if selected { theme.fg } else { theme.fg_dim })
            .bg(row_bg);
        let mut x = row_rect.x + 2;
        let spans = match &entries[index] {
            Entry::Existing {
                display, matched, ..
            } => highlighted_spans(
                display,
                |i| matched.binary_search(&i).is_ok(),
                row_rect.width.saturating_sub(3) as usize,
                base,
                base.fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Entry::Typed(value) => {
                x = put(
                    buf,
                    x,
                    row_rect.y,
                    row_rect.right(),
                    "Use new value: ",
                    base.fg(theme.fg_mute),
                );
                highlighted_spans(
                    value,
                    |_| false,
                    row_rect.right().saturating_sub(x + 1) as usize,
                    base.fg(theme.green),
                    base,
                )
            }
        };
        for span in spans {
            x = put(
                buf,
                x,
                row_rect.y,
                row_rect.right(),
                &span.content,
                span.style,
            );
        }
    }
    render_hints(
        buf,
        Rect::new(
            inner.x + 1,
            inner.bottom() - 1,
            inner.width.saturating_sub(1),
            1,
        ),
        &[
            hint("Enter", "save"),
            hint("↑↓", "select"),
            hint("Esc", "cancel"),
        ],
        theme,
        bg,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker(values: &[&str]) -> ValuePickerState {
        ValuePickerState::new(
            "t".into(),
            1,
            "city".into(),
            "TEXT".into(),
            values.iter().map(|v| v.to_string()).collect(),
            SqlValue::Null,
        )
    }

    #[test]
    fn typing_selects_the_best_match_not_the_typed_text() {
        let mut state = picker(&["Berlin", "Bern", "Paris"]);
        for ch in "berl".chars() {
            state.handle_key(&KeyEvent::from(KeyCode::Char(ch)));
        }
        assert_eq!(state.selected_value().as_deref(), Some("Berlin"));
    }

    #[test]
    fn a_new_value_is_offered_last() {
        let mut state = picker(&["Berlin"]);
        for ch in "Oslo".chars() {
            state.handle_key(&KeyEvent::from(KeyCode::Char(ch)));
        }
        assert_eq!(state.selected_value().as_deref(), Some("Oslo"));
        state.query.set("Ber");
        assert_eq!(state.entries().len(), 2);
        assert_eq!(state.selected_value().as_deref(), Some("Berlin"));
    }
}
