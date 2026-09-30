use crossterm::event::{KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::{layout::Rect, Frame};

use super::{
    search::{SearchChrome, SearchPanel},
    PopupAction,
};
use crate::{
    db::{
        schema::Column,
        types::{ColumnKind, SqlValue},
        SearchHit,
    },
    symbols::Symbols,
    theme::Theme,
    ui::widgets::hints::hint,
};

/// Candidate rows fetched per search of the referenced table.
pub const FK_PICKER_LIMIT: i64 = 200;

/// Chooses the row a foreign-key cell should reference. The referenced table is
/// searched in SQL, so every row is reachable however large the table is.
pub struct FkPickerState {
    pub target_table: String,
    /// The referenced key column first, then the descriptive columns.
    pub columns: Vec<Column>,
    pub source_table: String,
    pub source_col: String,
    pub source_rowid: i64,
    pub original: SqlValue,
    pub panel: SearchPanel,
}

impl FkPickerState {
    pub fn new(
        target_table: String,
        columns: Vec<Column>,
        source_table: String,
        source_col: String,
        source_rowid: i64,
        original: SqlValue,
    ) -> Self {
        Self {
            target_table,
            columns,
            source_table,
            source_col,
            source_rowid,
            original,
            panel: SearchPanel::loading(),
        }
    }

    pub fn query(&self) -> &str {
        self.panel.query.value()
    }

    pub fn set_hits(&mut self, hits: Vec<SearchHit>, symbols: &Symbols) {
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
            .set_results(headers, kinds, rows, FK_PICKER_LIMIT as usize, symbols);
    }

    /// The referenced key of the selected row.
    pub fn selected_value(&self) -> Option<SqlValue> {
        self.panel.table.selected_row()?.first().cloned()
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
    state: &mut FkPickerState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let subject = format!(
        "{} {} {}",
        state.source_col, symbols.foreign_key_arrow, state.target_table
    );
    let hints = [
        hint("Enter", "set value"),
        hint("↑↓", "select"),
        hint("Ctrl-←→", "columns"),
        hint("Esc", "cancel"),
    ];
    state.panel.render(
        frame,
        area,
        &SearchChrome {
            verb: "Link",
            subject: &subject,
            label: " Search: ",
            hints: &hints,
        },
        theme,
        symbols,
    );
}
