//! A search field over a result table, shared by Find, the foreign-key picker,
//! global search and the SQL console. Searching itself runs in the database;
//! this panel owns input, results, selection and presentation.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::{layout::Rect, style::Style, Frame};

use super::PopupAction;
use crate::{
    db::types::{ColumnKind, SqlValue},
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{render_hints, Hint},
        input::{render_labeled, TextInput},
        state::StateView,
        table::ResultTable,
        text::{group_thousands, put},
    },
};

/// The text around a search panel: title, input label and key hints.
pub struct SearchChrome<'a> {
    pub verb: &'a str,
    pub subject: &'a str,
    pub label: &'a str,
    pub hints: &'a [Hint],
}

#[derive(Debug, Default)]
pub struct SearchPanel {
    pub query: TextInput,
    pub table: ResultTable,
    pub loading: bool,
    pub error: Option<String>,
    /// Whether the result count reached the search limit.
    pub limited: bool,
}

impl SearchPanel {
    pub fn loading() -> Self {
        Self {
            loading: true,
            ..Self::default()
        }
    }

    pub fn set_results(
        &mut self,
        headers: Vec<String>,
        kinds: Vec<ColumnKind>,
        rows: Vec<Vec<SqlValue>>,
        limit: usize,
        symbols: &Symbols,
    ) {
        self.limited = rows.len() >= limit;
        self.table.set_highlight(self.query.value());
        self.table.set_data(headers, kinds, rows, symbols);
        self.loading = false;
        self.error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.loading = false;
        self.error = Some(error);
    }

    /// Esc closes, Enter submits a selected row, list keys move, anything else
    /// edits the query.
    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        match key.code {
            KeyCode::Esc => return PopupAction::Close,
            KeyCode::Enter if self.table.selected().is_some() => return PopupAction::Submit,
            KeyCode::Enter => return PopupAction::Handled,
            KeyCode::Home | KeyCode::End if !self.query.is_empty() => {}
            KeyCode::Left | KeyCode::Right if !key.modifiers.contains(KeyModifiers::CONTROL) => {}
            _ if self.table.handle_key(key) => return PopupAction::Handled,
            _ => {}
        }
        match self.query.handle_key(key) {
            crate::ui::widgets::input::InputOutcome::Changed => PopupAction::QueryChanged,
            crate::ui::widgets::input::InputOutcome::Moved => PopupAction::Handled,
            crate::ui::widgets::input::InputOutcome::Ignored => PopupAction::Ignored,
        }
    }

    pub fn handle_mouse(
        &mut self,
        kind: MouseEventKind,
        modifiers: KeyModifiers,
        x: u16,
        y: u16,
    ) -> PopupAction {
        match self.table.handle_mouse(kind, modifiers, x, y) {
            Some(true) => PopupAction::Submit,
            Some(false) => PopupAction::Handled,
            None => PopupAction::Ignored,
        }
    }

    pub fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        chrome: &SearchChrome<'_>,
        theme: &Theme,
        symbols: &Symbols,
    ) {
        let SearchChrome {
            verb,
            subject,
            label,
            hints,
        } = *chrome;
        let width = (area.width * 4 / 5).max(60);
        let height = (area.height * 4 / 5).max(12);
        let inner = PopupFrame::new(verb, Some(subject), width, height).render(frame, area, theme);
        if inner.height < 5 {
            return;
        }
        let bg = theme.bg_raised;
        let buf = frame.buffer_mut();
        render_labeled(
            buf,
            Rect::new(inner.x, inner.y, inner.width, 1),
            label,
            &self.query,
            theme,
            symbols,
        );

        let status = if self.loading {
            format!(" Searching{}", symbols.ellipsis)
        } else if self.limited {
            format!(
                " First {} matches, refine the search to see more",
                group_thousands(self.table.len() as i64)
            )
        } else {
            format!(" {} matches", group_thousands(self.table.len() as i64))
        };
        put(
            buf,
            inner.x,
            inner.y + 1,
            inner.right(),
            &status,
            Style::default().fg(theme.fg_mute).bg(bg),
        );

        let table_area = Rect::new(inner.x, inner.y + 2, inner.width, inner.height - 3);
        let empty = match (&self.error, self.loading) {
            (Some(error), _) => StateView::Error(error),
            (None, true) => StateView::Loading("Searching"),
            (None, false) => StateView::Empty("No matching rows", Some("Change the search text")),
        };
        self.table.render(buf, table_area, empty, theme, symbols);
        render_hints(
            buf,
            Rect::new(
                inner.x + 1,
                inner.bottom() - 1,
                inner.width.saturating_sub(1),
                1,
            ),
            hints,
            theme,
            bg,
        );
    }
}
