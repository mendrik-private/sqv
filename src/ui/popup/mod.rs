pub mod command_palette;
pub mod date_picker;
pub mod export;
pub mod filter;
pub mod find;
pub mod fk_picker;
pub mod global_search;
pub mod goto;
pub mod help;
pub mod insert_row;
pub mod json_view;
pub mod record;
pub mod references;
pub(crate) mod relative_time;
pub mod schema_view;
mod search;
pub mod sql_console;
pub mod text_editor;
pub mod value_picker;

use ratatui::{layout::Rect, Frame};

use crate::{symbols::Symbols, theme::Theme};

pub use command_palette::{CommandPaletteState, PaletteCommand};
pub use date_picker::DatePickerState;
pub use export::ExportState;
pub use filter::FilterPopupState;
pub use find::FindState;
pub use fk_picker::FkPickerState;
pub use global_search::GlobalSearchState;
pub use goto::GoToRowState;
pub use help::HelpState;
pub use insert_row::InsertRowState;
pub use json_view::JsonViewState;
pub use record::RecordState;
pub use references::ReferencesState;
pub use schema_view::SchemaViewState;
pub use sql_console::SqlConsoleState;
pub use text_editor::TextEditorState;
pub use value_picker::ValuePickerState;

/// What a popup made of a key or mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAction {
    /// Not a key this popup uses.
    Ignored,
    /// Consumed; only the popup's own state changed.
    Handled,
    Close,
    /// Confirm the popup's current choice.
    Submit,
    /// The search text changed and results must be reloaded.
    QueryChanged,
    /// Copy the popup's current value to the clipboard.
    Copy,
    /// Follow the link of the current item.
    Follow,
}

pub enum PopupKind {
    TextEditor(TextEditorState),
    ValuePicker(ValuePickerState),
    DatePicker(DatePickerState),
    InsertRow(InsertRowState),
    FkPicker(FkPickerState),
    FilterPopup(FilterPopupState),
    CommandPalette(CommandPaletteState),
    Help(HelpState),
    Find(FindState),
    GoToRow(GoToRowState),
    Record(RecordState),
    Schema(SchemaViewState),
    SqlConsole(SqlConsoleState),
    GlobalSearch(GlobalSearchState),
    Export(ExportState),
    References(ReferencesState),
    Json(JsonViewState),
}

pub fn render_popup(
    frame: &mut Frame,
    area: Rect,
    popup: &mut PopupKind,
    theme: &Theme,
    symbols: &Symbols,
) {
    match popup {
        PopupKind::TextEditor(state) => text_editor::render(frame, area, state, theme, symbols),
        PopupKind::ValuePicker(state) => value_picker::render(frame, area, state, theme, symbols),
        PopupKind::DatePicker(state) => date_picker::render(frame, area, state, theme, symbols),
        // Inserts are edited inline in the grid, which renders them.
        PopupKind::InsertRow(_) => {}
        PopupKind::FkPicker(state) => fk_picker::render(frame, area, state, theme, symbols),
        PopupKind::FilterPopup(state) => filter::render(frame, area, state, theme, symbols),
        PopupKind::CommandPalette(state) => {
            command_palette::render(frame, area, state, theme, symbols)
        }
        PopupKind::Help(state) => help::render(frame, area, state, theme, symbols),
        PopupKind::Find(state) => find::render(frame, area, state, theme, symbols),
        PopupKind::GoToRow(state) => goto::render(frame, area, state, theme, symbols),
        PopupKind::Record(state) => record::render(frame, area, state, theme, symbols),
        PopupKind::Schema(state) => schema_view::render(frame, area, state, theme, symbols),
        PopupKind::SqlConsole(state) => sql_console::render(frame, area, state, theme, symbols),
        PopupKind::GlobalSearch(state) => global_search::render(frame, area, state, theme, symbols),
        PopupKind::Export(state) => export::render(frame, area, state, theme, symbols),
        PopupKind::References(state) => references::render(frame, area, state, theme, symbols),
        PopupKind::Json(state) => json_view::render(frame, area, state, theme, symbols),
    }
}
