use std::cmp::Reverse;

use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthChar;

use crate::{db::types::SqlValue, symbols::Symbols, theme::Theme};

use super::search_result_format::format_search_result_text;

#[allow(dead_code)]
pub struct ValuePickerState {
    pub table: String,
    pub rowid: i64,
    pub col_name: String,
    pub col_type: String,
    pub values: Vec<String>,
    pub filter: String,
    pub selected: usize,
    pub original: SqlValue,
}

struct FilteredValue<'a> {
    raw: &'a str,
    display: String,
    score: i64,
    matched: Vec<usize>,
}

impl ValuePickerState {
    #[allow(dead_code)]
    pub fn new(
        table: String,
        rowid: i64,
        col_name: String,
        col_type: String,
        values: Vec<String>,
        original: SqlValue,
    ) -> Self {
        Self {
            table,
            rowid,
            col_name,
            col_type,
            values,
            filter: String::new(),
            selected: 0,
            original,
        }
    }

    pub fn selected_sql_value(&self) -> Option<SqlValue> {
        let value = self.selected_value()?;
        let upper = self.col_type.to_uppercase();
        if upper.contains("INT") {
            return value.parse::<i64>().ok().map(SqlValue::Integer);
        }
        if upper.contains("REAL") || upper.contains("FLOAT") || upper.contains("DOUBLE") {
            return value.parse::<f64>().ok().map(SqlValue::Real);
        }
        Some(SqlValue::Text(value.to_string()))
    }

    fn filtered_values(&self) -> Vec<FilteredValue<'_>> {
        if self.filter.is_empty() {
            return self
                .values
                .iter()
                .map(|value| FilteredValue {
                    raw: value.as_str(),
                    display: format_search_result_text(value),
                    score: 0,
                    matched: vec![],
                })
                .collect();
        }
        let matcher = SkimMatcherV2::default();
        let mut results: Vec<FilteredValue<'_>> = self
            .values
            .iter()
            .filter_map(|value| {
                let display = format_search_result_text(value);
                matcher
                    .fuzzy_indices(&display, &self.filter)
                    .map(|(score, indices)| FilteredValue {
                        raw: value.as_str(),
                        display,
                        score,
                        matched: indices,
                    })
            })
            .collect();
        results.sort_by_key(|result| Reverse(result.score));
        results
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        let total = self.option_count();
        if self.selected + 1 < total {
            self.selected += 1;
        }
    }

    pub fn selected_value(&self) -> Option<&str> {
        if self.selected_custom() {
            return Some(self.filter.as_str());
        }
        let filtered = self.filtered_values();
        let offset = usize::from(self.custom_value_available());
        filtered
            .get(self.selected.saturating_sub(offset))
            .map(|entry| entry.raw)
    }

    pub fn push_filter_char(&mut self, ch: char) {
        self.filter.push(ch);
        self.selected = 0;
    }

    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    fn custom_value_available(&self) -> bool {
        !self.filter.trim().is_empty() && self.is_textual()
    }

    fn selected_custom(&self) -> bool {
        self.custom_value_available() && self.selected == 0
    }

    fn option_count(&self) -> usize {
        self.display_entries().len()
    }

    fn display_entries(&self) -> Vec<(String, Vec<usize>, bool)> {
        let mut entries = Vec::new();
        if self.custom_value_available() {
            entries.push((self.filter.clone(), Vec::new(), true));
        }
        entries.extend(
            self.filtered_values()
                .into_iter()
                .map(|entry| (entry.display, entry.matched, false)),
        );
        entries
    }

    fn is_textual(&self) -> bool {
        let upper = self.col_type.to_uppercase();
        !(upper.contains("INT")
            || upper.contains("REAL")
            || upper.contains("FLOAT")
            || upper.contains("DOUBLE"))
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &ValuePickerState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let popup_width = (area.width / 2).max(30).min(area.width);
    let popup_height = 15u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    };

    super::paint_popup_surface(frame, popup_area, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(format!(" Pick: {} ", state.col_name))
        .style(Style::default().bg(theme.bg_raised));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let input_line = Line::from(vec![
        Span::styled(" New/Search: ", Style::default().fg(theme.fg_dim)),
        Span::styled(
            format!(" {} ", state.filter),
            Style::default().fg(theme.fg).bg(theme.bg_soft),
        ),
        Span::styled(
            symbols.cursor.to_string(),
            Style::default().fg(theme.accent).bg(theme.bg_soft),
        ),
    ]);

    let entries = state.display_entries();
    let list_height = inner.height.saturating_sub(4) as usize;
    let start = if state.selected >= list_height {
        state.selected - list_height + 1
    } else {
        0
    };

    let mut lines = vec![
        Line::from(Span::styled(
            " Type value here. Matching entries stay below.",
            Style::default().fg(theme.fg_faint),
        )),
        input_line,
        Line::from(""),
    ];
    for (rel_i, (val, matched, is_custom)) in
        entries.iter().skip(start).take(list_height).enumerate()
    {
        let abs_i = rel_i + start;
        let is_selected = abs_i == state.selected;
        let bg = if is_selected {
            theme.bg_soft
        } else {
            theme.bg_raised
        };

        let mut spans: Vec<Span> = Vec::new();
        if is_selected {
            spans.push(Span::styled(
                format!(" {} ", symbols.selection),
                Style::default().fg(theme.accent).bg(bg),
            ));
        } else {
            spans.push(Span::styled("   ", Style::default().bg(bg)));
        }
        if *is_custom {
            spans.push(Span::styled(
                "Use new value: ",
                Style::default().fg(theme.fg_dim).bg(bg),
            ));
            spans.extend(truncated_spans(
                val,
                &[],
                inner.width.saturating_sub(17) as usize,
                Style::default().fg(theme.green).bg(bg),
                Style::default().fg(theme.green).bg(bg),
            ));
        } else {
            spans.extend(truncated_spans(
                val,
                matched,
                inner.width.saturating_sub(4) as usize,
                Style::default().fg(theme.fg).bg(bg),
                Style::default().fg(theme.accent).bg(bg),
            ));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(Span::styled(
        format!(
            " {} select/add {} text field above creates new entry {} esc cancel",
            symbols.enter, symbols.separator, symbols.separator
        ),
        Style::default().fg(theme.fg_faint),
    )));

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.bg_raised)),
        inner,
    );
}

fn truncated_spans<'a>(
    value: &'a str,
    matched: &[usize],
    max_width: usize,
    base_style: Style,
    matched_style: Style,
) -> Vec<Span<'a>> {
    if max_width == 0 {
        return Vec::new();
    }

    let chars: Vec<(usize, char)> = value.chars().enumerate().collect();
    let total_width: usize = chars
        .iter()
        .map(|(_, ch)| UnicodeWidthChar::width(*ch).unwrap_or(1))
        .sum();
    let needs_ellipsis = total_width > max_width;
    let content_limit = if needs_ellipsis && max_width > 3 {
        max_width - 3
    } else {
        max_width
    };

    let mut spans = Vec::new();
    let mut used_width = 0usize;
    for (idx, ch) in chars {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(1);
        if used_width + ch_width > content_limit {
            break;
        }
        let style = if matched.contains(&idx) {
            matched_style
        } else {
            base_style
        };
        spans.push(Span::styled(ch.to_string(), style));
        used_width += ch_width;
    }

    if needs_ellipsis {
        spans.push(Span::styled("...".to_string(), base_style));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::truncated_spans;
    use ratatui::style::Style;

    #[test]
    fn truncates_long_values_with_ellipsis() {
        let spans = truncated_spans(
            "Embraer - Empresa Brasileira de Aeronáutica S.A.",
            &[],
            12,
            Style::default(),
            Style::default(),
        );
        let rendered = spans
            .into_iter()
            .map(|span| span.content)
            .collect::<String>();

        assert_eq!(rendered, "Embraer -...");
    }

    #[test]
    fn leaves_short_values_untouched() {
        let spans = truncated_spans("Apple Inc.", &[0], 20, Style::default(), Style::default());
        let rendered = spans
            .into_iter()
            .map(|span| span.content)
            .collect::<String>();

        assert_eq!(rendered, "Apple Inc.");
    }
}
