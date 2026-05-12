use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};

use crate::{
    db::{schema::Column, types::SqlValue},
    symbols::Symbols,
    theme::Theme,
};

use super::search_results_table::{
    formatted_search_result_value, render_search_result_table, SearchResultTable,
    SearchResultTableRow,
};

pub struct FindState {
    pub query: String,
    pub table_name: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<SqlValue>>,
    pub selected: usize,
    pub loading: bool,
}

pub struct FindHit {
    pub abs_row_index: usize,
    pub first_match_col: usize,
    pub matched_ranges: Vec<Option<(usize, usize)>>,
}

impl FindState {
    pub fn new(table_name: String, columns: Vec<Column>) -> Self {
        Self {
            query: String::new(),
            table_name,
            columns,
            rows: Vec::new(),
            selected: 0,
            loading: true,
        }
    }

    pub fn push_char(&mut self, ch: char) {
        self.query.push(ch);
        self.selected = 0;
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        let count = self.hit_count();
        if self.selected + 1 < count {
            self.selected += 1;
        }
    }

    pub fn visible_hits(&self) -> Vec<FindHit> {
        if self.query.trim().is_empty() {
            return self
                .rows
                .iter()
                .enumerate()
                .map(|(idx, row)| FindHit {
                    abs_row_index: idx,
                    first_match_col: 0,
                    matched_ranges: vec![None; row.len()],
                })
                .collect();
        }

        let needle = self.query.trim().to_lowercase();
        let needle_chars = needle.chars().count();
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let mut found = false;
                let mut first_match_col = 0;
                let mut matched_ranges = Vec::with_capacity(row.len());
                for (col_idx, value) in row.iter().enumerate() {
                    let display = formatted_search_result_value(value);
                    let lower = display.to_lowercase();
                    let range = lower.find(&needle).map(|byte_start| {
                        let start = lower[..byte_start].chars().count();
                        if !found {
                            found = true;
                            first_match_col = col_idx;
                        }
                        (start, start + needle_chars)
                    });
                    matched_ranges.push(range);
                }
                found.then_some(FindHit {
                    abs_row_index: idx,
                    first_match_col,
                    matched_ranges,
                })
            })
            .collect()
    }

    fn hit_count(&self) -> usize {
        if self.query.trim().is_empty() {
            self.rows.len()
        } else {
            self.visible_hits().len()
        }
    }

    fn column_headers(&self) -> Vec<String> {
        self.columns
            .iter()
            .map(|column| column.name.clone())
            .collect()
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &FindState, theme: &Theme, symbols: &Symbols) {
    let popup_width = ((area.width * 4) / 5).max(60).min(area.width);
    let popup_height = ((area.height * 3) / 5).max(12).min(area.height);
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
        .title(format!(" Find in {} ", state.table_name))
        .style(Style::default().bg(theme.bg_raised));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    if state.loading {
        frame.render_widget(
            Paragraph::new(symbols.loading_label("Loading"))
                .style(Style::default().fg(theme.fg_dim).bg(theme.bg_raised)),
            inner,
        );
        return;
    }

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let hint_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let input_area = Rect {
        x: inner.x,
        y: inner.y.saturating_add(1),
        width: inner.width,
        height: 1,
    };
    let footer_area = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Type to filter rows. Enter jumps to the selected match.",
            Style::default().fg(theme.fg_faint),
        )))
        .style(Style::default().bg(theme.bg_raised)),
        hint_area,
    );

    let input_line = Line::from(vec![
        Span::styled(" Search: ", Style::default().fg(theme.fg_dim)),
        Span::styled(
            format!(" {} ", state.query),
            Style::default().fg(theme.fg).bg(theme.bg_soft),
        ),
        Span::styled(
            symbols.cursor.to_string(),
            Style::default().fg(theme.accent).bg(theme.bg_soft),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(input_line).style(Style::default().bg(theme.bg_raised)),
        input_area,
    );

    let hits = state.visible_hits();
    let headers = state.column_headers();
    let visible_columns: Vec<_> = (0..headers.len()).collect();
    let table_rows: Vec<_> = hits
        .iter()
        .map(|hit| SearchResultTableRow {
            row_index: hit.abs_row_index,
            matched_ranges: &hit.matched_ranges,
        })
        .collect();

    let table_top_y = inner.y + 2;
    if footer_area.y > table_top_y {
        render_search_result_table(
            frame.buffer_mut(),
            Rect {
                x: popup_area.x,
                y: table_top_y,
                width: popup_area.width,
                height: footer_area.y - table_top_y,
            },
            SearchResultTable {
                headers: &headers,
                rows: &state.rows,
                visible_rows: &table_rows,
                selected: state.selected,
                visible_columns: &visible_columns,
            },
            theme,
            symbols,
        );
    }

    let count_text = if state.query.trim().is_empty() {
        format!(" {} rows", state.rows.len())
    } else {
        format!(" {} of {} matched", hits.len(), state.rows.len())
    };
    let footer_line = Line::from(vec![
        Span::styled(count_text, Style::default().fg(theme.fg_mute)),
        Span::styled(
            format!(
                "  {} go {} {}{} select {} esc cancel",
                symbols.enter,
                symbols.separator,
                symbols.arrow_up,
                symbols.arrow_down,
                symbols.separator
            ),
            Style::default().fg(theme.fg_faint),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(footer_line).style(Style::default().bg(theme.bg_raised)),
        footer_area,
    );
}
