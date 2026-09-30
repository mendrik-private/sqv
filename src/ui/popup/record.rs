use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEventKind};
use ratatui::{
    layout::{Position, Rect},
    style::{Modifier, Style},
    Frame,
};

use super::PopupAction;
use crate::{
    db::types::{ColumnKind, SqlValue},
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        cell::cell_text,
        frame::PopupFrame,
        hints::{hint, render_hints, Hint},
        list::{paint_row, ListCursor},
        scrollbar::Scrollbar,
        text::{put, sanitize, text_width, truncate_with_ellipsis},
    },
};

/// One row shown vertically, one field per line, with the selected value in
/// full below. Wide rows become readable without scrolling sideways.
pub struct RecordState {
    pub table: String,
    pub row_number: i64,
    pub names: Vec<String>,
    pub kinds: Vec<ColumnKind>,
    pub values: Vec<SqlValue>,
    pub links: Vec<bool>,
    pub editable: bool,
    pub fields: ListCursor,
    list_area: Rect,
}

/// What a record view shows: the row, its columns and where to start.
pub struct RecordInit {
    pub table: String,
    pub row_number: i64,
    pub names: Vec<String>,
    pub kinds: Vec<ColumnKind>,
    pub values: Vec<SqlValue>,
    pub links: Vec<bool>,
    pub editable: bool,
    pub focused: usize,
}

impl RecordState {
    pub fn new(init: RecordInit) -> Self {
        let RecordInit {
            table,
            row_number,
            names,
            kinds,
            values,
            links,
            editable,
            focused,
        } = init;
        let mut fields = ListCursor::default();
        fields.select(focused, names.len());
        Self {
            table,
            row_number,
            names,
            kinds,
            values,
            links,
            editable,
            fields,
            list_area: Rect::default(),
        }
    }

    pub fn selected(&self) -> usize {
        self.fields.selected
    }

    pub fn selected_value(&self) -> Option<&SqlValue> {
        self.values.get(self.fields.selected)
    }

    /// Whether the selected value parses as a JSON object or array.
    pub fn selected_is_json(&self) -> bool {
        matches!(self.selected_value(), Some(SqlValue::Text(text)) if is_json_document(text))
    }

    /// Enter edits the field (`Submit`), `j` follows a link, `y` copies, `o`
    /// opens a JSON value in the viewer (`Follow` on a non-link JSON field).
    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        let on_link = self
            .links
            .get(self.fields.selected)
            .copied()
            .unwrap_or(false);
        match key.code {
            KeyCode::Esc | KeyCode::Char('v') => PopupAction::Close,
            KeyCode::Enter if self.editable => PopupAction::Submit,
            KeyCode::Char('j') if on_link => PopupAction::Follow,
            KeyCode::Char('o') if self.selected_is_json() => PopupAction::Follow,
            KeyCode::Char('y') => PopupAction::Copy,
            KeyCode::Char('k') => {
                self.fields.move_by(-1, self.names.len());
                PopupAction::Handled
            }
            KeyCode::Char('j') => {
                self.fields.move_by(1, self.names.len());
                PopupAction::Handled
            }
            _ if self.fields.handle_key(key, self.names.len()) => PopupAction::Handled,
            _ => PopupAction::Ignored,
        }
    }

    pub fn handle_mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> PopupAction {
        let len = self.names.len();
        match kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                self.fields.scroll(kind == MouseEventKind::ScrollDown, len);
                PopupAction::Handled
            }
            MouseEventKind::Down(MouseButton::Left)
                if self.list_area.contains(Position { x, y }) =>
            {
                if let Some(index) = self.fields.hit((y - self.list_area.y) as usize, len) {
                    self.fields.select(index, len);
                }
                PopupAction::Handled
            }
            _ => PopupAction::Ignored,
        }
    }

    fn hints(&self) -> Vec<Hint> {
        let mut hints = Vec::new();
        if self.editable {
            hints.push(hint("Enter", "edit"));
        }
        if self
            .links
            .get(self.fields.selected)
            .copied()
            .unwrap_or(false)
        {
            hints.push(hint("j", "follow link"));
        }
        if self.selected_is_json() {
            hints.push(hint("o", "view JSON"));
        }
        hints.extend([hint("y", "copy"), hint("Esc", "close")]);
        hints
    }
}

pub fn is_json_document(text: &str) -> bool {
    let trimmed = text.trim_start();
    (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(text).is_ok()
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut RecordState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let subject = format!("{} · row {}", state.table, state.row_number);
    let inner = PopupFrame::new(
        "Record",
        Some(&subject),
        (area.width * 7 / 10).max(50),
        (area.height * 8 / 10).max(12),
    )
    .render(frame, area, theme);
    if inner.height < 6 {
        return;
    }
    let bg = theme.bg_raised;
    let buf = frame.buffer_mut();
    let detail_height = (inner.height / 3).clamp(3, 10);
    let list = Rect::new(
        inner.x,
        inner.y,
        inner.width.saturating_sub(1),
        inner.height - detail_height - 2,
    );
    state.list_area = list;
    let name_width = state
        .names
        .iter()
        .map(|name| text_width(name))
        .max()
        .unwrap_or(0)
        .clamp(4, (list.width / 3) as usize);
    let len = state.names.len();
    for (row, index) in state.fields.visible(len, list.height as usize).enumerate() {
        let y = list.y + row as u16;
        let selected = index == state.fields.selected;
        paint_row(
            buf,
            Rect::new(list.x, y, list.width, 1),
            selected,
            theme,
            symbols,
        );
        let row_bg = if selected { theme.bg_soft } else { bg };
        let name = truncate_with_ellipsis(&state.names[index], name_width, symbols.ellipsis);
        let name_style = Style::default()
            .fg(theme.fg_mute)
            .bg(row_bg)
            .add_modifier(Modifier::BOLD);
        put(buf, list.x + 2, y, list.right(), &name, name_style);
        let value_x = list.x + 3 + name_width as u16;
        let marker = if state.links[index] {
            symbols.foreign_key_arrow.to_string()
        } else {
            String::new()
        };
        let (text, _) = cell_text(&state.values[index], state.kinds[index], symbols);
        let room = list.right().saturating_sub(value_x + 2) as usize;
        let value = truncate_with_ellipsis(&format!("{marker}{text}"), room, symbols.ellipsis);
        let value_style = match state.values[index] {
            SqlValue::Null => Style::default()
                .fg(theme.fg_faint)
                .add_modifier(Modifier::ITALIC),
            _ => Style::default().fg(if selected { theme.fg } else { theme.fg_dim }),
        };
        put(
            buf,
            value_x,
            y,
            list.right(),
            &value,
            value_style.bg(row_bg),
        );
    }
    Scrollbar {
        offset: state.fields.offset(),
        total: len,
        viewport: list.height as usize,
    }
    .render(
        buf,
        Rect::new(inner.right() - 1, list.y, 1, list.height),
        bg,
        theme,
        symbols,
    );

    // Full value of the selected field, wrapped.
    let rule_y = list.bottom();
    let rule: String = symbols
        .box_horizontal
        .to_string()
        .repeat(inner.width as usize);
    put(
        buf,
        inner.x,
        rule_y,
        inner.right(),
        &rule,
        Style::default().fg(theme.line).bg(bg),
    );
    if let Some(value) = state.selected_value() {
        let text = match value {
            SqlValue::Text(text) if is_json_document(text) => {
                serde_json::from_str::<serde_json::Value>(text)
                    .and_then(|json| serde_json::to_string_pretty(&json))
                    .unwrap_or_else(|_| text.clone())
            }
            value => value.to_text().into_owned(),
        };
        let width = inner.width.saturating_sub(2).max(1) as usize;
        let mut lines = Vec::new();
        for line in text.lines() {
            let chars: Vec<char> = sanitize(line).chars().collect();
            if chars.is_empty() {
                lines.push(String::new());
            }
            for chunk in chars.chunks(width) {
                lines.push(chunk.iter().collect::<String>());
            }
        }
        for (row, line) in lines.iter().take(detail_height as usize).enumerate() {
            put(
                buf,
                inner.x + 1,
                rule_y + 1 + row as u16,
                inner.right(),
                line,
                Style::default().fg(theme.fg).bg(bg),
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
        &state.hints(),
        theme,
        bg,
    );
}
