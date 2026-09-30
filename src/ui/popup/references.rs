use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEventKind};
use ratatui::{
    layout::{Position, Rect},
    style::Style,
    Frame,
};

use super::PopupAction;
use crate::{
    db::types::SqlValue,
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{hint, render_hints},
        list::{paint_row, ListCursor},
        state::{render_state, StateView},
        text::{put, sanitize},
    },
};

/// A foreign key elsewhere that can point at the focused row.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub table: String,
    pub column: String,
    /// The value of the referenced key in the focused row.
    pub value: SqlValue,
}

/// Lists the tables and columns that reference the focused row; choosing one
/// opens that table filtered to the referencing rows.
pub struct ReferencesState {
    pub table: String,
    pub references: Vec<Reference>,
    pub list: ListCursor,
    list_area: Rect,
}

impl ReferencesState {
    pub fn new(table: String, references: Vec<Reference>) -> Self {
        Self {
            table,
            references,
            list: ListCursor::default(),
            list_area: Rect::default(),
        }
    }

    pub fn selected(&self) -> Option<&Reference> {
        self.references.get(self.list.selected)
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        match key.code {
            KeyCode::Esc => PopupAction::Close,
            KeyCode::Enter if self.selected().is_some() => PopupAction::Submit,
            _ if self.list.handle_key(key, self.references.len()) => PopupAction::Handled,
            _ => PopupAction::Ignored,
        }
    }

    pub fn handle_mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> PopupAction {
        let len = self.references.len();
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
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut ReferencesState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let height = (state.references.len() as u16 + 4).clamp(6, 20);
    let inner = PopupFrame::new("Referencing rows", Some(&state.table), 60, height)
        .render(frame, area, theme);
    if inner.height < 3 {
        return;
    }
    let bg = theme.bg_raised;
    let buf = frame.buffer_mut();
    let list = Rect::new(inner.x, inner.y, inner.width, inner.height - 1);
    state.list_area = list;
    if state.references.is_empty() {
        render_state(
            buf,
            list,
            StateView::Empty("No table references this one", None),
            bg,
            theme,
            symbols,
        );
    }
    for (row, index) in state
        .list
        .visible(state.references.len(), list.height as usize)
        .enumerate()
    {
        let reference = &state.references[index];
        let y = list.y + row as u16;
        let selected = index == state.list.selected;
        paint_row(
            buf,
            Rect::new(list.x, y, list.width, 1),
            selected,
            theme,
            symbols,
        );
        let row_bg = if selected { theme.bg_soft } else { bg };
        let text = format!(
            "{}.{} = {}",
            sanitize(&reference.table),
            sanitize(&reference.column),
            reference.value.to_text()
        );
        put(
            buf,
            list.x + 2,
            y,
            list.right(),
            &text,
            Style::default()
                .fg(if selected { theme.fg } else { theme.fg_dim })
                .bg(row_bg),
        );
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
            hint("Enter", "open filtered"),
            hint("↑↓", "select"),
            hint("Esc", "close"),
        ],
        theme,
        bg,
    );
}
