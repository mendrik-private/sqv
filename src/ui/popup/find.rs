use crossterm::event::{KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::{layout::Rect, Frame};

use super::{
    search::{SearchChrome, SearchPanel},
    PopupAction,
};
use crate::{
    db::{schema::Column, types::ColumnKind, SearchHit},
    symbols::Symbols,
    theme::Theme,
    ui::widgets::hints::hint,
};

/// Rows fetched per search; the panel says when a search hit the limit.
pub const FIND_LIMIT: i64 = 500;

/// Finds rows of the current view whose values contain the search text. The
/// search runs in SQL over the whole view; Enter jumps to the selected row.
pub struct FindState {
    pub table_name: String,
    pub columns: Vec<Column>,
    pub panel: SearchPanel,
    offsets: Vec<i64>,
}

impl FindState {
    pub fn new(table_name: String, columns: Vec<Column>) -> Self {
        Self {
            table_name,
            columns,
            panel: SearchPanel::loading(),
            offsets: Vec::new(),
        }
    }

    pub fn query(&self) -> &str {
        self.panel.query.value()
    }

    pub fn set_hits(&mut self, hits: Vec<SearchHit>, symbols: &Symbols) {
        self.offsets = hits.iter().map(|hit| hit.offset).collect();
        let headers = self
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        let kinds = self
            .columns
            .iter()
            .map(|column| ColumnKind::of(&column.col_type, &column.name))
            .collect();
        let rows = hits.into_iter().map(|hit| hit.values).collect();
        self.panel
            .set_results(headers, kinds, rows, FIND_LIMIT as usize, symbols);
    }

    /// The view offset of the selected match.
    pub fn selected_offset(&self) -> Option<i64> {
        self.offsets.get(self.panel.table.selected()?).copied()
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        self.panel.handle_key(key)
    }

    pub fn handle_mouse(
        &mut self,
        kind: MouseEventKind,
        modifiers: KeyModifiers,
        x: u16,
        y: u16,
    ) -> PopupAction {
        self.panel.handle_mouse(kind, modifiers, x, y)
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut FindState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let hints = [
        hint("Enter", "go to row"),
        hint("↑↓", "select"),
        hint("Ctrl-←→", "columns"),
        hint("Esc", "close"),
    ];
    state.panel.render(
        frame,
        area,
        &SearchChrome {
            verb: "Find",
            subject: &state.table_name,
            label: " Contains: ",
            hints: &hints,
        },
        theme,
        symbols,
    );
}
