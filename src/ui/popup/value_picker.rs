use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};

use crate::{
    db::types::{expects_number, parse_input, SqlValue},
    symbols::Symbols,
    theme::Theme,
};

use super::{
    fuzzy_filter, highlighted_spans, search_result_format::format_search_result_text, FuzzyMatch,
};

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

impl ValuePickerState {
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
        parse_input(&self.col_type, self.selected_value()?).ok()
    }

    /// Values matching the filter as (raw value, display text, matched indices).
    fn filtered_values(&self) -> Vec<FuzzyMatch<&str>> {
        fuzzy_filter(
            self.values
                .iter()
                .map(|value| (value.as_str(), format_search_result_text(value))),
            &self.filter,
        )
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
            .map(|(raw, _, _)| *raw)
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
                .map(|(_, display, matched)| (display, matched, false)),
        );
        entries
    }

    fn is_textual(&self) -> bool {
        !expects_number(&self.col_type)
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
    let popup_area = super::centered_rect(area, popup_width, popup_height);

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
            spans.extend(highlighted_spans(
                val,
                |_| false,
                inner.width.saturating_sub(17) as usize,
                Style::default().fg(theme.green).bg(bg),
                Style::default().fg(theme.green).bg(bg),
            ));
        } else {
            spans.extend(highlighted_spans(
                val,
                |idx| matched.binary_search(&idx).is_ok(),
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
