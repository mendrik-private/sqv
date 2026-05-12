use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
};
use unicode_width::UnicodeWidthChar;

use crate::{
    db::{
        schema::Column,
        types::{affinity, ColAffinity, SqlValue},
    },
    grid::layout,
    symbols::Symbols,
    theme::Theme,
};

use super::search_result_format::format_search_result_text;

const SELECT_COL_WIDTH: usize = 2;

#[derive(Clone, Copy)]
struct HighlightStyles {
    base: Style,
    matched: Style,
}

#[derive(Clone, Copy)]
pub(crate) struct TableRuleGlyphs {
    pub(crate) middle: char,
}

pub(crate) struct SearchResultTableRow<'a> {
    pub(crate) row_index: usize,
    pub(crate) matched_ranges: &'a [Option<(usize, usize)>],
}

pub(crate) struct SearchResultTable<'a> {
    pub(crate) headers: &'a [String],
    pub(crate) rows: &'a [Vec<SqlValue>],
    pub(crate) visible_rows: &'a [SearchResultTableRow<'a>],
    pub(crate) selected: usize,
    pub(crate) visible_columns: &'a [usize],
}

pub(crate) fn render_search_result_table(
    buf: &mut Buffer,
    table_area: Rect,
    table: SearchResultTable<'_>,
    theme: &Theme,
    symbols: &Symbols,
) {
    let SearchResultTable {
        headers,
        rows,
        visible_rows,
        selected,
        visible_columns,
    } = table;

    if table_area.width <= 2 || table_area.height < 3 || visible_columns.is_empty() {
        return;
    }

    let inner_available = table_area
        .width
        .saturating_sub(2)
        .saturating_sub(SELECT_COL_WIDTH as u16) as usize;
    if inner_available == 0 {
        return;
    }

    let initial_data_width = inner_available.saturating_sub(visible_columns.len());
    let all_widths = compute_column_widths(
        headers,
        rows,
        visible_rows,
        visible_columns,
        initial_data_width,
    );
    let fit_count = clipped_visible_count(&all_widths, inner_available);
    let visible_columns = &visible_columns[..fit_count];
    let data_width = inner_available.saturating_sub(visible_columns.len());
    let column_widths =
        compute_column_widths(headers, rows, visible_rows, visible_columns, data_width);
    let separator_positions =
        compute_separator_positions(table_area.x + 1, &column_widths, SELECT_COL_WIDTH);

    render_table_rule(
        buf,
        table_area,
        table_area.y,
        &separator_positions,
        TableRuleGlyphs {
            middle: symbols.table_rule_header_mid,
        },
        theme,
        symbols,
    );

    let header_y = table_area.y + 1;
    fill_row(
        buf,
        Rect {
            x: table_area.x + 1,
            y: header_y,
            width: table_area.width.saturating_sub(2),
            height: 1,
        },
        Style::default().bg(theme.bg_raised),
    );
    render_vertical_separators(
        buf,
        header_y,
        &separator_positions,
        theme,
        theme.bg_raised,
        symbols,
    );
    let mut cell_x = table_area.x + 1 + SELECT_COL_WIDTH as u16 + 1;
    for (&col_idx, &width) in visible_columns.iter().zip(&column_widths) {
        draw_truncated_text(
            buf,
            cell_x + 1,
            header_y,
            width.saturating_sub(1),
            &headers[col_idx],
            None,
            HighlightStyles {
                base: Style::default()
                    .fg(theme.fg)
                    .bg(theme.bg_raised)
                    .add_modifier(Modifier::BOLD),
                matched: Style::default()
                    .fg(theme.fg)
                    .bg(theme.bg_raised)
                    .add_modifier(Modifier::BOLD),
            },
        );
        cell_x += width as u16 + 1;
    }

    render_table_rule(
        buf,
        table_area,
        table_area.y + 2,
        &separator_positions,
        TableRuleGlyphs {
            middle: symbols.table_rule_cross_mid,
        },
        theme,
        symbols,
    );

    let visible_result_rows = table_area.height.saturating_sub(3) as usize;
    if visible_result_rows == 0 {
        return;
    }

    let start = if selected >= visible_result_rows {
        selected - visible_result_rows + 1
    } else {
        0
    };

    if visible_rows.is_empty() {
        let row_y = table_area.y + 3;
        fill_row(
            buf,
            Rect {
                x: table_area.x + 1,
                y: row_y,
                width: table_area.width.saturating_sub(2),
                height: 1,
            },
            Style::default().bg(theme.bg_raised),
        );
        render_vertical_separators(
            buf,
            row_y,
            &separator_positions,
            theme,
            theme.bg_raised,
            symbols,
        );
        draw_truncated_text(
            buf,
            table_area.x + 1 + SELECT_COL_WIDTH as u16 + 2,
            row_y,
            table_area
                .width
                .saturating_sub(SELECT_COL_WIDTH as u16)
                .saturating_sub(visible_columns.len() as u16)
                .saturating_sub(4) as usize,
            "No matches",
            None,
            HighlightStyles {
                base: Style::default().fg(theme.fg_faint).bg(theme.bg_raised),
                matched: Style::default().fg(theme.fg_faint).bg(theme.bg_raised),
            },
        );
        return;
    }

    for (view_idx, hit) in visible_rows
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_result_rows)
    {
        let row_y = table_area.y + 3 + (view_idx - start) as u16;
        let is_selected = view_idx == selected;
        let bg = if is_selected {
            theme.bg_soft
        } else {
            theme.bg_raised
        };
        fill_row(
            buf,
            Rect {
                x: table_area.x + 1,
                y: row_y,
                width: table_area.width.saturating_sub(2),
                height: 1,
            },
            Style::default().bg(bg),
        );
        render_vertical_separators(buf, row_y, &separator_positions, theme, bg, symbols);
        if is_selected {
            buf.set_string(
                table_area.x + 1,
                row_y,
                symbols.selection.to_string(),
                Style::default().fg(theme.accent).bg(bg),
            );
        }

        let row = &rows[hit.row_index];
        let mut cell_x = table_area.x + 1 + SELECT_COL_WIDTH as u16 + 1;
        for (&col_idx, &width) in visible_columns.iter().zip(&column_widths) {
            let display = row
                .get(col_idx)
                .map(formatted_search_result_value)
                .unwrap_or_else(|| symbols.missing_placeholder.to_string());
            draw_truncated_text(
                buf,
                cell_x + 1,
                row_y,
                width.saturating_sub(1),
                &display,
                hit.matched_ranges.get(col_idx).copied().flatten(),
                HighlightStyles {
                    base: Style::default()
                        .fg(if is_selected { theme.fg } else { theme.fg_dim })
                        .bg(bg),
                    matched: Style::default()
                        .fg(theme.accent)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                },
            );
            cell_x += width as u16 + 1;
        }
    }
}

pub(crate) fn render_table_rule(
    buf: &mut Buffer,
    table_area: Rect,
    y: u16,
    separator_positions: &[u16],
    glyphs: TableRuleGlyphs,
    theme: &Theme,
    symbols: &Symbols,
) {
    let left_x = table_area.x.saturating_add(1);
    let right_x = table_area.x + table_area.width.saturating_sub(2);
    if right_x < left_x {
        return;
    }

    for x in left_x..=right_x {
        let symbol = if separator_positions.contains(&x) {
            glyphs.middle
        } else {
            symbols.box_horizontal
        };
        buf.set_string(
            x,
            y,
            symbol.to_string(),
            Style::default().fg(theme.line).bg(theme.bg_raised),
        );
    }
}

pub(crate) fn formatted_search_result_value(value: &SqlValue) -> String {
    match value {
        SqlValue::Null => "null".to_string(),
        SqlValue::Integer(n) => n.to_string(),
        SqlValue::Real(f) => f.to_string(),
        SqlValue::Text(text) => format_search_result_text(text),
        SqlValue::Blob(bytes) => format!("<blob {} bytes>", bytes.len()),
    }
}

pub(crate) fn value_is_numeric_or_temporal(value: &SqlValue) -> bool {
    match value {
        SqlValue::Integer(_) | SqlValue::Real(_) => true,
        SqlValue::Text(text) => format_search_result_text(text) != *text,
        SqlValue::Null | SqlValue::Blob(_) => false,
    }
}

fn compute_column_widths(
    headers: &[String],
    rows: &[Vec<SqlValue>],
    visible_rows: &[SearchResultTableRow<'_>],
    visible_columns: &[usize],
    available_width: usize,
) -> Vec<usize> {
    if visible_columns.is_empty() || available_width == 0 {
        return Vec::new();
    }
    if visible_columns.len() == 1 {
        return vec![available_width];
    }

    let columns: Vec<Column> = visible_columns
        .iter()
        .map(|&col_idx| synth_column(headers[col_idx].clone(), rows, col_idx))
        .collect();
    let rendered_rows: Vec<Vec<SqlValue>> = visible_rows
        .iter()
        .map(|hit| {
            visible_columns
                .iter()
                .filter_map(|&col_idx| rows[hit.row_index].get(col_idx).cloned())
                .collect()
        })
        .collect();
    let fk_cols = vec![false; columns.len()];
    let widths = layout::compute_col_widths(
        &columns,
        &rendered_rows,
        available_width.min(u16::MAX as usize) as u16,
        &HashMap::new(),
        &fk_cols,
    );
    distribute_extra_width_like_grid(columns, widths, available_width)
}

fn clipped_visible_count(widths: &[usize], inner_available: usize) -> usize {
    let mut used = 0usize;
    let mut count = 0usize;
    for &width in widths {
        let needed = used + width + count + 1;
        if count == 0 || needed <= inner_available {
            used += width;
            count += 1;
        } else {
            break;
        }
    }
    count.max(1).min(widths.len())
}

fn compute_separator_positions(
    mut left: u16,
    column_widths: &[usize],
    select_width: usize,
) -> Vec<u16> {
    let mut separators = Vec::with_capacity(column_widths.len());
    left += select_width as u16;
    separators.push(left);
    left += 1;
    for (index, &width) in column_widths.iter().enumerate() {
        left += width as u16;
        if index + 1 < column_widths.len() {
            separators.push(left);
            left += 1;
        }
    }
    separators
}

fn render_vertical_separators(
    buf: &mut Buffer,
    y: u16,
    separator_positions: &[u16],
    theme: &Theme,
    bg: Color,
    symbols: &Symbols,
) {
    for &x in separator_positions {
        buf.set_string(
            x,
            y,
            symbols.box_vertical.to_string(),
            Style::default().fg(theme.line).bg(bg),
        );
    }
}

fn fill_row(buf: &mut Buffer, area: Rect, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    buf.set_string(area.x, area.y, " ".repeat(area.width as usize), style);
}

fn draw_truncated_text(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    max_width: usize,
    value: &str,
    matched_range: Option<(usize, usize)>,
    styles: HighlightStyles,
) {
    let mut cursor = x;
    for span in truncated_spans(value, matched_range, max_width, styles.base, styles.matched) {
        for ch in span.content.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            buf.set_string(cursor, y, ch.to_string(), span.style);
            cursor += ch_width;
        }
    }
}

fn truncated_spans<'a>(
    value: &'a str,
    matched_range: Option<(usize, usize)>,
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
        let style = if matched_range.is_some_and(|(start, end)| idx >= start && idx < end) {
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

fn synth_column(header: String, rows: &[Vec<SqlValue>], col_idx: usize) -> Column {
    Column {
        cid: col_idx as i64,
        name: header,
        col_type: inferred_column_type(rows, col_idx),
        not_null: false,
        default_value: None,
        is_pk: col_idx == 0,
    }
}

fn inferred_column_type(rows: &[Vec<SqlValue>], col_idx: usize) -> String {
    for value in rows.iter().filter_map(|row| row.get(col_idx)) {
        match value {
            SqlValue::Integer(_) => return "INTEGER".to_string(),
            SqlValue::Real(_) => return "REAL".to_string(),
            SqlValue::Text(_) | SqlValue::Null | SqlValue::Blob(_) => {}
        }
    }
    "TEXT".to_string()
}

fn distribute_extra_width_like_grid(
    columns: Vec<Column>,
    widths: Vec<u16>,
    available_width: usize,
) -> Vec<usize> {
    let mut widths: Vec<usize> = widths.into_iter().map(usize::from).collect();
    let total: usize = widths.iter().sum();
    if total >= available_width || widths.is_empty() {
        return widths;
    }

    let extra = available_width - total;
    let text_cols: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter_map(|(idx, column)| {
            matches!(
                affinity(&column.col_type),
                ColAffinity::Text | ColAffinity::Blob
            )
            .then_some(idx)
        })
        .collect();

    let targets = if text_cols.is_empty() {
        vec![widths.len() - 1]
    } else {
        text_cols
    };

    let base = extra / targets.len();
    let remainder = extra % targets.len();
    for (idx, target) in targets.into_iter().enumerate() {
        widths[target] += base + usize::from(idx < remainder);
    }
    widths
}
