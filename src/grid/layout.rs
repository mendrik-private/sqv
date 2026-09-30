//! Content-aware column widths.

use crate::{
    db::types::{ColumnKind, SqlValue},
    symbols::Symbols,
    ui::widgets::{cell::cell_text, text::text_width},
};

/// One space either side of the cell content.
pub const CELL_PADDING: u16 = 2;
const MAX_TEXT_WIDTH: usize = 40;
const MAX_DESIRED_WIDTH: usize = 80;
const MIN_WIDTH: u16 = 6;

/// What each column needs: `widths` are the chosen widths, `desired` how wide a
/// column would be to show its sampled values in full.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColumnWidths {
    pub widths: Vec<u16>,
    pub desired: Vec<u16>,
}

/// The header of one column: name, whether a sort marker is shown, and the
/// rendered metadata line (type badge, key markers, filter marker).
pub struct HeaderNeeds<'a> {
    pub name: &'a str,
    pub sort_marker_width: usize,
    pub meta: &'a str,
}

/// Sizes every column from `rows` (a stable sample of the table). Numbers and
/// dates must never be cut, so they size to their widest value; free text sizes
/// to the 80th percentile of non-NULL values so one long value does not dominate.
pub fn compute_col_widths(
    kinds: &[ColumnKind],
    headers: &[HeaderNeeds<'_>],
    rows: &[Vec<SqlValue>],
    symbols: &Symbols,
) -> ColumnWidths {
    let mut result = ColumnWidths::default();
    for (col, (kind, header)) in kinds.iter().zip(headers).enumerate() {
        let mut values: Vec<usize> = rows
            .iter()
            .filter_map(|row| row.get(col))
            .filter(|value| **value != SqlValue::Null)
            .map(|value| text_width(&cell_text(value, *kind, symbols).0))
            .collect();
        values.sort_unstable();
        let widest = values.last().copied().unwrap_or(0);
        let content = if kind.is_numeric() || kind.is_temporal() || *kind == ColumnKind::Boolean {
            widest
        } else if values.is_empty() {
            0
        } else {
            values[(values.len() - 1) * 8 / 10].min(MAX_TEXT_WIDTH)
        };
        let nulls = values.len() < rows.len();
        let content = content.max(if nulls { text_width("NULL") } else { 0 });
        let header_width = text_width(header.name) + header.sort_marker_width + 1;
        let meta_width = text_width(header.meta);
        let width =
            (content.max(header_width).max(meta_width) as u16 + CELL_PADDING).max(MIN_WIDTH);
        let desired = (widest.min(MAX_DESIRED_WIDTH) as u16 + CELL_PADDING).max(width);
        result.widths.push(width);
        result.desired.push(desired);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(kind: ColumnKind, name: &str, values: &[SqlValue]) -> (u16, u16) {
        let symbols = Symbols::default_with_nerd_font(false);
        let rows: Vec<Vec<SqlValue>> = values.iter().map(|v| vec![v.clone()]).collect();
        let result = compute_col_widths(
            &[kind],
            &[HeaderNeeds {
                name,
                sort_marker_width: 0,
                meta: "INT",
            }],
            &rows,
            &symbols,
        );
        (result.widths[0], result.desired[0])
    }

    #[test]
    fn numbers_size_to_their_widest_value() {
        let values: Vec<SqlValue> = (0..50)
            .map(|i| SqlValue::Integer(if i < 45 { 1_000_000 } else { 11_170_334_000 }))
            .collect();
        assert_eq!(
            widths(ColumnKind::Integer, "Bytes", &values).0,
            11 + CELL_PADDING
        );
    }

    #[test]
    fn text_uses_a_percentile_and_ignores_nulls() {
        let mut values = vec![SqlValue::Null; 40];
        values.extend((0..10).map(|i| {
            SqlValue::Text(if i == 9 {
                "x".repeat(60)
            } else {
                "abcdef".into()
            })
        }));
        let (width, desired) = widths(ColumnKind::Text, "c", &values);
        assert_eq!(width, 6 + CELL_PADDING);
        assert_eq!(desired, 60 + CELL_PADDING);
    }

    #[test]
    fn header_names_always_fit_with_their_padding() {
        let (width, _) = widths(ColumnKind::Integer, "CustomerId", &[SqlValue::Integer(1)]);
        assert_eq!(
            width as usize,
            "CustomerId".len() + 1 + CELL_PADDING as usize
        );
    }
}
