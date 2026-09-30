//! Cell values as text: the grid, result tables and previews all format values
//! here so a value looks the same wherever it appears.

use std::borrow::Cow;

use chrono::DateTime;

use super::text::{sanitize, text_width, truncate_with_ellipsis};
use crate::{
    db::types::{ColumnKind, SqlValue},
    symbols::Symbols,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// The display text of `value` in a column of `kind`, and how to align it.
pub fn cell_text<'a>(
    value: &'a SqlValue,
    kind: ColumnKind,
    symbols: &Symbols,
) -> (Cow<'a, str>, Align) {
    match value {
        SqlValue::Null => (Cow::Borrowed("NULL"), Align::Left),
        SqlValue::Integer(n) => match kind {
            ColumnKind::Boolean if *n == 0 || *n == 1 => {
                let glyph = if *n == 1 {
                    symbols.bool_true
                } else {
                    symbols.bool_false
                };
                (Cow::Owned(glyph.to_string()), Align::Center)
            }
            ColumnKind::EpochDatetime => match epoch_text(*n) {
                Some(text) => (Cow::Owned(text), Align::Left),
                None => (Cow::Owned(n.to_string()), Align::Right),
            },
            _ => (Cow::Owned(n.to_string()), Align::Right),
        },
        SqlValue::Real(f) => {
            let scale = match kind {
                ColumnKind::Real { scale } | ColumnKind::Numeric { scale } => scale,
                _ => None,
            };
            (Cow::Owned(real_text(*f, scale)), Align::Right)
        }
        SqlValue::Text(text) => (sanitize(text), Align::Left),
        SqlValue::Blob(_) => (value.to_text(), Align::Left),
    }
}

/// Reals use the declared scale when there is one, otherwise the shortest text
/// that reads back as the same number.
pub fn real_text(value: f64, scale: Option<u8>) -> String {
    match scale {
        Some(scale) => format!("{value:.*}", scale as usize),
        None => value.to_string(),
    }
}

fn epoch_text(value: i64) -> Option<String> {
    let dt = if value.abs() > 100_000_000_000 {
        DateTime::from_timestamp_millis(value)?
    } else {
        DateTime::from_timestamp(value, 0)?
    };
    Some(dt.format("%Y-%m-%d %H:%M:%S").to_string())
}

/// Fits cell text into `width` cells. Text gets an ellipsis; numbers are never
/// shortened into a different number and fill the cell with `#` instead.
pub fn fit_cell(text: &str, width: usize, align: Align, ellipsis: char) -> String {
    if text_width(text) <= width {
        return text.to_string();
    }
    if align == Align::Right {
        return "#".repeat(width);
    }
    truncate_with_ellipsis(text, width, ellipsis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_that_do_not_fit_are_never_shown_truncated() {
        assert_eq!(fit_cell("11170334", 7, Align::Right, '…'), "#######");
        assert_eq!(fit_cell("Embraer Empresa", 8, Align::Left, '…'), "Embraer…");
    }

    #[test]
    fn reals_use_declared_scale_or_the_shortest_form() {
        assert_eq!(real_text(0.99, None), "0.99");
        assert_eq!(real_text(13.86, Some(2)), "13.86");
        assert_eq!(real_text(1.5, Some(3)), "1.500");
    }

    #[test]
    fn epoch_columns_show_dates_and_text_is_sanitised() {
        let symbols = Symbols::default_with_nerd_font(false);
        let (text, _) = cell_text(&SqlValue::Integer(0), ColumnKind::EpochDatetime, &symbols);
        assert_eq!(text, "1970-01-01 00:00:00");
        let value = SqlValue::Text("a\nb".into());
        assert_eq!(cell_text(&value, ColumnKind::Text, &symbols).0, "a↵b");
    }
}
