use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};

use crate::{db::types::SqlValue, symbols::Symbols, theme::Theme};

use super::search_results_table::{
    formatted_search_result_value, render_search_result_table, value_is_numeric_or_temporal,
    SearchResultTable, SearchResultTableRow,
};

pub struct FkPickerState {
    pub target_table: String,
    #[allow(dead_code)]
    pub target_col: String,
    pub display_cols: Vec<String>,
    pub rows: Vec<Vec<SqlValue>>,
    pub filter: String,
    pub selected: usize,
    pub source_table: String,
    pub source_col: String,
    pub source_rowid: i64,
    pub loading: bool,
    pub original: crate::db::types::SqlValue,
}

pub struct RowHit {
    pub row_index: usize,
    pub matched_ranges: Vec<Option<(usize, usize)>>,
}

impl FkPickerState {
    pub fn new(
        target_table: String,
        target_col: String,
        display_cols: Vec<String>,
        source_table: String,
        source_col: String,
        source_rowid: i64,
        original: crate::db::types::SqlValue,
    ) -> Self {
        Self {
            target_table,
            target_col,
            display_cols,
            rows: Vec::new(),
            filter: String::new(),
            selected: 0,
            source_table,
            source_col,
            source_rowid,
            loading: true,
            original,
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        let visible = self.visible_rows();
        if self.selected + 1 < visible.len() {
            self.selected += 1;
        }
    }

    pub fn push_filter_char(&mut self, ch: char) {
        self.filter.push(ch);
        self.selected = 0;
    }

    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    pub fn selected_value(&self) -> Option<&SqlValue> {
        let hit = self.visible_rows().get(self.selected)?.row_index;
        self.rows.get(hit)?.first()
    }

    pub fn visible_rows(&self) -> Vec<RowHit> {
        if self.filter.trim().is_empty() {
            return self
                .rows
                .iter()
                .enumerate()
                .map(|(row_index, row)| RowHit {
                    row_index,
                    matched_ranges: vec![None; row.len()],
                })
                .collect();
        }

        let needle = self.filter.trim().to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(row_index, row)| {
                self.match_row(row, &needle).map(|matched_ranges| RowHit {
                    row_index,
                    matched_ranges,
                })
            })
            .collect()
    }

    fn visible_column_indices(&self, visible_rows: &[RowHit]) -> Vec<usize> {
        let total_columns = self.column_headers().len();
        let numeric_search = search_prefers_numeric(&self.filter);
        let allowed = |col_idx: usize| {
            col_idx == 0 || numeric_search || !is_numeric_or_temporal_column(self, col_idx)
        };

        if self.filter.trim().is_empty() || visible_rows.is_empty() {
            let columns: Vec<usize> = (0..total_columns)
                .filter(|&col_idx| allowed(col_idx))
                .collect();
            return if columns.is_empty() { vec![0] } else { columns };
        }

        let last_matched = visible_rows
            .iter()
            .flat_map(|row| {
                row.matched_ranges
                    .iter()
                    .enumerate()
                    .filter_map(|(col_idx, matched)| matched.map(|_| col_idx))
            })
            .max();

        if let Some(last_matched) = last_matched {
            let columns: Vec<usize> = (0..=last_matched)
                .filter(|&col_idx| allowed(col_idx))
                .collect();
            if columns.is_empty() {
                vec![0]
            } else {
                columns
            }
        } else {
            let columns: Vec<usize> = (0..total_columns)
                .filter(|&col_idx| allowed(col_idx))
                .collect();
            if columns.is_empty() {
                vec![0]
            } else {
                columns
            }
        }
    }

    fn column_headers(&self) -> Vec<String> {
        let mut headers = Vec::with_capacity(self.display_cols.len() + 1);
        headers.push(self.target_col.clone());
        headers.extend(self.display_cols.iter().cloned());
        headers
    }

    fn match_row(&self, row: &[SqlValue], needle: &str) -> Option<Vec<Option<(usize, usize)>>> {
        let needle_len = needle.chars().count();
        let mut matched_ranges = Vec::with_capacity(row.len());
        let mut found = false;

        for value in row {
            let display = formatted_search_result_value(value);
            let display_lower = display.to_lowercase();
            let matched = display_lower.find(needle).map(|byte_start| {
                let start = display_lower[..byte_start].chars().count();
                found = true;
                (start, start + needle_len)
            });
            matched_ranges.push(matched);
        }

        found.then_some(matched_ranges)
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &FkPickerState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let popup_width = ((area.width * 3) / 5).max(54).min(area.width);
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
        .title(format!(
            " FK: {} {} {} ",
            state.source_col, symbols.foreign_key_arrow, state.target_table
        ))
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
            " Type to filter related rows. Matching columns stay in view.",
            Style::default().fg(theme.fg_faint),
        )))
        .style(Style::default().bg(theme.bg_raised)),
        hint_area,
    );

    let filter_line = Line::from(vec![
        Span::styled(" Search: ", Style::default().fg(theme.fg_dim)),
        Span::styled(
            format!(" {} ", state.filter),
            Style::default().fg(theme.fg).bg(theme.bg_soft),
        ),
        Span::styled(
            symbols.cursor.to_string(),
            Style::default().fg(theme.accent).bg(theme.bg_soft),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(filter_line).style(Style::default().bg(theme.bg_raised)),
        input_area,
    );

    let table_top_y = inner.y + 2;
    if footer_area.y > table_top_y {
        let visible_rows = state.visible_rows();
        let visible_columns = state.visible_column_indices(&visible_rows);
        let table_rows: Vec<_> = visible_rows
            .iter()
            .map(|row| SearchResultTableRow {
                row_index: row.row_index,
                matched_ranges: &row.matched_ranges,
            })
            .collect();
        render_result_table(
            frame.buffer_mut(),
            Rect {
                x: popup_area.x,
                y: table_top_y,
                width: popup_area.width,
                height: footer_area.y - table_top_y,
            },
            state,
            &table_rows,
            &visible_columns,
            theme,
            symbols,
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {} select {} esc cancel", symbols.enter, symbols.separator),
            Style::default().fg(theme.fg_faint),
        )))
        .style(Style::default().bg(theme.bg_raised)),
        footer_area,
    );
}

fn render_result_table(
    buf: &mut ratatui::buffer::Buffer,
    table_area: Rect,
    state: &FkPickerState,
    visible_rows: &[SearchResultTableRow<'_>],
    visible_columns: &[usize],
    theme: &Theme,
    symbols: &Symbols,
) {
    let headers = state.column_headers();
    render_search_result_table(
        buf,
        table_area,
        SearchResultTable {
            headers: &headers,
            rows: &state.rows,
            visible_rows,
            selected: state.selected,
            visible_columns,
        },
        theme,
        symbols,
    );
}

fn search_prefers_numeric(filter: &str) -> bool {
    filter
        .trim_start()
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
}

fn is_numeric_or_temporal_column(state: &FkPickerState, col_idx: usize) -> bool {
    state
        .rows
        .iter()
        .filter_map(|row| row.get(col_idx))
        .any(value_is_numeric_or_temporal)
}

#[cfg(test)]
mod tests {
    use super::{is_numeric_or_temporal_column, FkPickerState};
    use crate::ui::popup::search_results_table::{render_table_rule, TableRuleGlyphs};
    use crate::{db::types::SqlValue, symbols::Symbols, theme::Theme};
    use ratatui::{buffer::Buffer, layout::Rect};

    #[test]
    fn fk_picker_matches_full_substrings_only() {
        let mut state = FkPickerState::new(
            "users".to_string(),
            "id".to_string(),
            vec!["name".to_string()],
            "posts".to_string(),
            "user_id".to_string(),
            1,
            SqlValue::Integer(1),
        );
        state.loading = false;
        state.rows = vec![
            vec![SqlValue::Integer(1), SqlValue::Text("abc".to_string())],
            vec![SqlValue::Integer(2), SqlValue::Text("axbyc".to_string())],
            vec![SqlValue::Integer(3), SqlValue::Text("zabcx".to_string())],
        ];
        state.filter = "abc".to_string();

        let rows = state.visible_rows();
        let ids = rows
            .iter()
            .filter_map(|hit| match &state.rows[hit.row_index][0] {
                SqlValue::Integer(id) => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn fk_picker_hides_non_matching_columns() {
        let mut state = FkPickerState::new(
            "users".to_string(),
            "id".to_string(),
            vec!["name".to_string(), "city".to_string()],
            "posts".to_string(),
            "user_id".to_string(),
            1,
            SqlValue::Integer(1),
        );
        state.loading = false;
        state.rows = vec![vec![
            SqlValue::Integer(1),
            SqlValue::Text("Alice".to_string()),
            SqlValue::Text("Berlin".to_string()),
        ]];
        state.filter = "ber".to_string();

        let rows = state.visible_rows();
        let visible = state.visible_column_indices(&rows);

        assert_eq!(visible, vec![0, 1, 2]);
    }

    #[test]
    fn fk_table_rules_stay_inside_popup_frame() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 4));
        let theme = Theme::default();
        let symbols = Symbols::default_with_nerd_font(true);

        render_table_rule(
            &mut buf,
            Rect::new(0, 0, 12, 4),
            0,
            &[3, 7],
            TableRuleGlyphs { middle: '┬' },
            &theme,
            &symbols,
        );

        let left = buf.cell((0, 0)).unwrap();
        let middle = buf.cell((3, 0)).unwrap();
        let right = buf.cell((11, 0)).unwrap();
        let line = buf.cell((1, 0)).unwrap();

        assert_eq!(left.symbol(), " ");
        assert_eq!(middle.symbol(), "┬");
        assert_eq!(right.symbol(), " ");
        assert_eq!(middle.fg, theme.line);
        assert_eq!(line.fg, theme.line);
    }

    #[test]
    fn empty_match_set_falls_back_to_all_columns() {
        let state = FkPickerState::new(
            "users".to_string(),
            "id".to_string(),
            vec!["name".to_string(), "city".to_string()],
            "posts".to_string(),
            "user_id".to_string(),
            1,
            SqlValue::Integer(1),
        );

        let visible = state.visible_column_indices(&[]);

        assert_eq!(visible, vec![0, 1, 2]);
    }

    #[test]
    fn hides_numeric_and_temporal_columns_until_search_starts_with_number() {
        let mut state = FkPickerState::new(
            "users".to_string(),
            "id".to_string(),
            vec!["name".to_string(), "created_at".to_string()],
            "posts".to_string(),
            "user_id".to_string(),
            1,
            SqlValue::Integer(1),
        );
        state.rows = vec![vec![
            SqlValue::Integer(42),
            SqlValue::Text("Alice".to_string()),
            SqlValue::Text("2026-04-25".to_string()),
        ]];

        assert!(is_numeric_or_temporal_column(&state, 0));
        assert!(!is_numeric_or_temporal_column(&state, 1));
        assert!(is_numeric_or_temporal_column(&state, 2));
        assert_eq!(
            state.visible_column_indices(&state.visible_rows()),
            vec![0, 1]
        );

        state.filter = "2".to_string();

        assert_eq!(
            state.visible_column_indices(&state.visible_rows()),
            vec![0, 1, 2]
        );
    }
}
