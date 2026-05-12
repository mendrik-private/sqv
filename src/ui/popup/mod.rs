pub mod command_palette;
pub mod date_picker;
pub mod datetime_picker;
pub mod filter;
pub mod find;
pub mod fk_picker;
pub mod help;
pub mod insert_row;
mod search_result_format;
mod search_results_table;
pub mod text_editor;
pub mod value_picker;

use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Clear},
    Frame,
};

use crate::{symbols::Symbols, theme::Theme};

pub use command_palette::{CommandPaletteState, PaletteCommand};
pub use date_picker::{DateFocus, DatePickerState};
pub use datetime_picker::{DatetimeFocus, DatetimePickerState};
pub use filter::FilterPopupState;
pub use find::FindState;
pub use fk_picker::FkPickerState;
pub use help::HelpState;
pub use insert_row::InsertRowState;
pub use text_editor::TextEditorState;
pub use value_picker::ValuePickerState;

#[allow(dead_code)]
pub enum PopupKind {
    TextEditor(TextEditorState),
    ValuePicker(ValuePickerState),
    DatePicker(DatePickerState),
    DatetimePicker(DatetimePickerState),
    InsertRow(InsertRowState),
    FkPicker(FkPickerState),
    FilterPopup(FilterPopupState),
    CommandPalette(CommandPaletteState),
    Help(HelpState),
    Find(FindState),
}

pub(crate) fn paint_popup_surface(frame: &mut Frame, area: Rect, theme: &Theme) {
    let shadow_area = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(1),
        height: area.height.saturating_sub(1),
    };
    if shadow_area.width > 0 && shadow_area.height > 0 {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.line_soft)),
            shadow_area,
        );
    }
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.bg_raised)),
        area,
    );
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
        PopupKind::DatetimePicker(state) => {
            datetime_picker::render(frame, area, state, theme, symbols)
        }
        PopupKind::InsertRow(state) => insert_row::render(frame, area, state, theme, symbols),
        PopupKind::FkPicker(state) => fk_picker::render(frame, area, state, theme, symbols),
        PopupKind::FilterPopup(state) => filter::render(frame, area, state, theme, symbols),
        PopupKind::CommandPalette(state) => {
            command_palette::render(frame, area, state, theme, symbols)
        }
        PopupKind::Help(state) => help::render(frame, area, state, theme, symbols),
        PopupKind::Find(state) => find::render(frame, area, state, theme, symbols),
    }
}
