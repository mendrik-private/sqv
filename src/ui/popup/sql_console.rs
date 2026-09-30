use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::{layout::Rect, style::Style, Frame};

use super::PopupAction;
use crate::{
    db::types::{ColumnKind, SqlValue},
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{hint, render_hints},
        input::{render_labeled, TextInput},
        state::StateView,
        table::ResultTable,
        text::{group_thousands, put},
    },
};

/// Rows shown for a query in the console.
pub const SQL_ROW_LIMIT: usize = 1000;
const HISTORY_LIMIT: usize = 200;

/// One-statement SQL console with a result table and a persistent history.
pub struct SqlConsoleState {
    pub input: TextInput,
    pub history: Vec<String>,
    history_position: Option<usize>,
    pub results: ResultTable,
    pub status: Option<(String, bool)>,
    pub running: bool,
}

impl SqlConsoleState {
    pub fn new(history: Vec<String>) -> Self {
        Self {
            input: TextInput::default(),
            history,
            history_position: None,
            results: ResultTable::default(),
            status: None,
            running: false,
        }
    }

    /// Records the statement about to run and returns it.
    pub fn take_statement(&mut self) -> String {
        let statement = self.input.value().trim().to_string();
        if !statement.is_empty() && self.history.last() != Some(&statement) {
            self.history.push(statement.clone());
            if self.history.len() > HISTORY_LIMIT {
                self.history.remove(0);
            }
        }
        self.history_position = None;
        self.running = true;
        statement
    }

    pub fn set_rows(
        &mut self,
        columns: Vec<String>,
        rows: Vec<Vec<SqlValue>>,
        truncated: bool,
        symbols: &Symbols,
    ) {
        let count = rows.len();
        let kinds = vec![ColumnKind::Untyped; columns.len()];
        self.results.set_data(columns, kinds, rows, symbols);
        let message = if truncated {
            format!("First {} rows", group_thousands(SQL_ROW_LIMIT as i64))
        } else {
            format!("{} rows", group_thousands(count as i64))
        };
        self.status = Some((message, false));
        self.running = false;
    }

    pub fn set_changed(&mut self, changed: usize) {
        self.status = Some((
            format!("{} rows changed", group_thousands(changed as i64)),
            false,
        ));
        self.running = false;
    }

    pub fn set_error(&mut self, error: String) {
        self.status = Some((error, true));
        self.running = false;
    }

    fn recall(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() - 1;
        let position = match (self.history_position, older) {
            (None, true) => Some(last),
            (None, false) => None,
            (Some(p), true) => Some(p.saturating_sub(1)),
            (Some(p), false) if p >= last => None,
            (Some(p), false) => Some(p + 1),
        };
        self.history_position = position;
        match position {
            Some(p) => self.input.set(self.history[p].clone()),
            None => self.input.clear(),
        }
    }

    /// Enter runs the statement (`Submit`); Ctrl-P / Ctrl-N walk the history;
    /// list keys move in the results.
    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return PopupAction::Close,
            KeyCode::Enter if !self.running => return PopupAction::Submit,
            KeyCode::Char('p') if ctrl => {
                self.recall(true);
                return PopupAction::Handled;
            }
            KeyCode::Char('n') if ctrl => {
                self.recall(false);
                return PopupAction::Handled;
            }
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                self.results.handle_key(key);
                return PopupAction::Handled;
            }
            KeyCode::Left | KeyCode::Right if ctrl => {
                self.results.handle_key(key);
                return PopupAction::Handled;
            }
            _ => {}
        }
        if self.input.handle_key(key).consumed() {
            PopupAction::Handled
        } else {
            PopupAction::Ignored
        }
    }

    pub fn handle_mouse(
        &mut self,
        kind: MouseEventKind,
        modifiers: KeyModifiers,
        x: u16,
        y: u16,
    ) -> PopupAction {
        match self.results.handle_mouse(kind, modifiers, x, y) {
            Some(_) => PopupAction::Handled,
            None => PopupAction::Ignored,
        }
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut SqlConsoleState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let inner = PopupFrame::new(
        "SQL console",
        None,
        (area.width * 9 / 10).max(60),
        (area.height * 8 / 10).max(12),
    )
    .render(frame, area, theme);
    if inner.height < 5 {
        return;
    }
    let bg = theme.bg_raised;
    let buf = frame.buffer_mut();
    render_labeled(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        " sql> ",
        &state.input,
        theme,
        symbols,
    );
    let (status, error) = match (&state.status, state.running) {
        (_, true) => (format!("Running{}", symbols.ellipsis), false),
        (Some((text, error)), false) => (text.clone(), *error),
        (None, false) => (
            "One statement at a time; queries show their rows".to_string(),
            false,
        ),
    };
    let color = if error { theme.red } else { theme.fg_mute };
    put(
        buf,
        inner.x + 1,
        inner.y + 1,
        inner.right(),
        &status,
        Style::default().fg(color).bg(bg),
    );
    let table = Rect::new(inner.x, inner.y + 2, inner.width, inner.height - 3);
    state.results.render(
        buf,
        table,
        StateView::Empty("No rows", None),
        theme,
        symbols,
    );
    render_hints(
        buf,
        Rect::new(
            inner.x + 1,
            inner.bottom() - 1,
            inner.width.saturating_sub(1),
            1,
        ),
        &[
            hint("Enter", "run"),
            hint("Ctrl-P/N", "history"),
            hint("↑↓", "rows"),
            hint("Ctrl-←→", "columns"),
            hint("Esc", "close"),
        ],
        theme,
        bg,
    );
}
