pub mod command_palette;
pub mod date_picker;
pub mod filter;
pub mod find;
pub mod fk_picker;
pub mod help;
pub mod insert_row;
mod search_result_format;
mod search_results_table;
mod text_cursor;
pub mod text_editor;
pub mod value_picker;

use std::cmp::Reverse;

use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use ratatui::{
    layout::Rect,
    style::Style,
    text::Span,
    widgets::{Block, Clear},
    Frame,
};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{symbols::Symbols, theme::Theme};

pub use command_palette::{CommandPaletteState, PaletteCommand};
pub use date_picker::{DateFocus, DatePickerState};
pub use filter::FilterPopupState;
pub use find::FindState;
pub use fk_picker::FkPickerState;
pub use help::HelpState;
pub use insert_row::InsertRowState;
pub use text_editor::TextEditorState;
pub use value_picker::ValuePickerState;

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
}

/// A fuzzy-filtered entry: the item, its label and the matched char indices.
pub(crate) type FuzzyMatch<T> = (T, String, Vec<usize>);

/// Items whose label fuzzy-matches `query`, best match first; every item in its
/// original order when the query is empty.
pub(crate) fn fuzzy_filter<T>(
    items: impl IntoIterator<Item = (T, String)>,
    query: &str,
) -> Vec<FuzzyMatch<T>> {
    if query.is_empty() {
        return items
            .into_iter()
            .map(|(item, label)| (item, label, Vec::new()))
            .collect();
    }
    let matcher = SkimMatcherV2::default();
    let mut scored = items
        .into_iter()
        .filter_map(|(item, label)| {
            let (score, matched) = matcher.fuzzy_indices(&label, query)?;
            Some((Reverse(score), (item, label, matched)))
        })
        .collect::<Vec<_>>();
    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().map(|(_, entry)| entry).collect()
}

/// One span per char of `value`, styled as matched where `is_matched(char index)`
/// holds, cut to `max_width` cells with a trailing `...` when it does not fit.
pub(crate) fn highlighted_spans(
    value: &str,
    is_matched: impl Fn(usize) -> bool,
    max_width: usize,
    base_style: Style,
    matched_style: Style,
) -> Vec<Span<'static>> {
    if max_width == 0 {
        return Vec::new();
    }
    let needs_ellipsis = UnicodeWidthStr::width(value) > max_width;
    let content_limit = if needs_ellipsis && max_width > 3 {
        max_width - 3
    } else {
        max_width
    };

    let mut spans = Vec::new();
    let mut used_width = 0usize;
    for (idx, ch) in value.chars().enumerate() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(1);
        if used_width + ch_width > content_limit {
            break;
        }
        let style = if is_matched(idx) {
            matched_style
        } else {
            base_style
        };
        spans.push(Span::styled(ch.to_string(), style));
        used_width += ch_width;
    }
    if needs_ellipsis {
        spans.push(Span::styled("...", base_style));
    }
    spans
}

/// A `width` x `height` rectangle centered in `area`, shrunk to fit it.
pub(crate) fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
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
        // Inserts are edited inline in the grid, which renders them.
        PopupKind::InsertRow(_) => {}
        PopupKind::FkPicker(state) => fk_picker::render(frame, area, state, theme, symbols),
        PopupKind::FilterPopup(state) => filter::render(frame, area, state, theme, symbols),
        PopupKind::CommandPalette(state) => {
            command_palette::render(frame, area, state, theme, symbols)
        }
        PopupKind::Help(state) => help::render(frame, area, state, theme, symbols),
        PopupKind::Find(state) => find::render(frame, area, state, theme, symbols),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::highlighted_spans;

    fn rendered(value: &str, max_width: usize) -> String {
        highlighted_spans(
            value,
            |_| false,
            max_width,
            Style::default(),
            Style::default(),
        )
        .into_iter()
        .map(|span| span.content)
        .collect()
    }

    #[test]
    fn truncates_long_values_with_ellipsis() {
        assert_eq!(
            rendered("Embraer - Empresa Brasileira de Aeronáutica S.A.", 12),
            "Embraer -..."
        );
    }

    #[test]
    fn leaves_short_values_untouched() {
        assert_eq!(rendered("Apple Inc.", 20), "Apple Inc.");
    }
}
