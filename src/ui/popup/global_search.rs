use crossterm::event::{KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::{layout::Rect, Frame};

use super::{
    search::{SearchChrome, SearchPanel},
    PopupAction,
};
use crate::{
    db::types::{ColumnKind, SqlValue},
    symbols::Symbols,
    theme::Theme,
    ui::widgets::hints::hint,
};

/// Matches per table in a search across all tables.
pub const GLOBAL_SEARCH_PER_TABLE: i64 = 20;
/// Searches start once the text is this long, to keep scans meaningful.
pub const GLOBAL_SEARCH_MIN_CHARS: usize = 2;

/// Where a global search match lives.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalHit {
    pub table: String,
    pub column: String,
    pub rowid: Option<i64>,
    pub value: SqlValue,
}

/// Searches the text columns of every table for the typed text.
pub struct GlobalSearchState {
    pub panel: SearchPanel,
    hits: Vec<GlobalHit>,
}

impl GlobalSearchState {
    pub fn new() -> Self {
        Self {
            panel: SearchPanel::default(),
            hits: Vec::new(),
        }
    }

    pub fn query(&self) -> &str {
        self.panel.query.value()
    }

    pub fn set_hits(&mut self, hits: Vec<GlobalHit>, limit: usize, symbols: &Symbols) {
        let rows = hits
            .iter()
            .map(|hit| {
                vec![
                    SqlValue::Text(hit.table.clone()),
                    SqlValue::Text(hit.column.clone()),
                    hit.value.clone(),
                ]
            })
            .collect();
        self.hits = hits;
        self.panel.set_results(
            vec!["Table".into(), "Column".into(), "Value".into()],
            vec![ColumnKind::Text, ColumnKind::Text, ColumnKind::Untyped],
            rows,
            limit,
            symbols,
        );
    }

    pub fn selected(&self) -> Option<&GlobalHit> {
        self.hits.get(self.panel.table.selected()?)
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

impl Default for GlobalSearchState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut GlobalSearchState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let hints = [
        hint("Enter", "open row"),
        hint("↑↓", "select"),
        hint("Esc", "close"),
    ];
    state.panel.render(
        frame,
        area,
        &SearchChrome {
            verb: "Search",
            subject: "all tables",
            label: " Contains: ",
            hints: &hints,
        },
        theme,
        symbols,
    );
}
