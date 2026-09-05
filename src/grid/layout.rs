use std::collections::HashMap;

use unicode_width::UnicodeWidthStr;

use crate::db::{
    schema::Column,
    types::{affinity, ColAffinity, SqlValue},
};

const CELL_SIDE_PADDING: u16 = 2;
const WIDTH_SAMPLE_TRIM: usize = 3;
const MAX_HEURISTIC_COL_WIDTH: u16 = 40;

fn cell_width(val: &SqlValue) -> u16 {
    match val {
        SqlValue::Null => 4,
        SqlValue::Integer(n) => n.to_string().len() as u16,
        SqlValue::Real(f) => format!("{:.6}", f).len() as u16,
        SqlValue::Text(s) => UnicodeWidthStr::width(s.as_str()) as u16,
        SqlValue::Blob(b) => format!("<blob {} bytes>", b.len()).len() as u16,
    }
}

fn heuristic_width(mut widths: Vec<u16>) -> u16 {
    if widths.is_empty() {
        return 0;
    }
    widths.sort_unstable();
    let kept = if widths.len() > WIDTH_SAMPLE_TRIM * 2 {
        &widths[WIDTH_SAMPLE_TRIM..widths.len() - WIDTH_SAMPLE_TRIM]
    } else {
        &widths[..]
    };
    let sum: u32 = kept.iter().map(|&w| w as u32).sum();
    let avg = ((sum + (kept.len() as u32 / 2)) / kept.len() as u32) as u16;
    avg.saturating_add(CELL_SIDE_PADDING)
        .min(MAX_HEURISTIC_COL_WIDTH)
}

pub fn compute_col_widths(
    columns: &[Column],
    rows: &[Vec<SqlValue>],
    avail_width: u16,
    manual_widths: &HashMap<usize, u16>,
    fk_cols: &[bool],
) -> Vec<u16> {
    let n = columns.len();
    if n == 0 {
        return Vec::new();
    }
    if avail_width == 0 {
        return vec![0u16; n];
    }

    let mut preferred = vec![0u16; n];

    for (i, col) in columns.iter().enumerate() {
        if manual_widths.contains_key(&i) {
            let w = manual_widths[&i];
            preferred[i] = w;
            continue;
        }

        let sample_widths: Vec<u16> = rows
            .iter()
            .filter_map(|r| r.get(i))
            .map(cell_width)
            .collect();
        let estimated_w = heuristic_width(sample_widths);

        let col_affinity = affinity(&col.col_type);
        let is_pk = col.is_pk;
        let is_fk = fk_cols.get(i).copied().unwrap_or(false);
        let name_width = UnicodeWidthStr::width(col.name.as_str()) as u16;
        let meta_label = if is_pk {
            "INT key"
        } else if is_fk {
            "INT link"
        } else {
            match col_affinity {
                ColAffinity::Integer => "INT",
                ColAffinity::Real => "REAL",
                ColAffinity::Text => "TXT",
                ColAffinity::Blob => "BLOB",
                ColAffinity::Numeric => "NUM",
            }
        };
        let meta_width = UnicodeWidthStr::width(meta_label) as u16 + 2;
        let header_name_w = name_width + 2;
        let min_width_i = header_name_w.max(meta_width).max(6);
        let preferred_i = estimated_w.max(min_width_i);

        preferred[i] = preferred_i;
    }

    let mut result = vec![0u16; n];
    for (&i, &w) in manual_widths {
        if i < n {
            result[i] = w;
        }
    }

    for (i, width) in preferred.into_iter().enumerate() {
        if !manual_widths.contains_key(&i) {
            result[i] = width;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema::Column, types::SqlValue};
    use std::collections::HashMap;

    fn make_col(name: &str, col_type: &str, is_pk: bool) -> Column {
        Column {
            cid: 0,
            name: name.to_string(),
            col_type: col_type.to_string(),
            not_null: false,
            default_value: None,
            is_pk,
            pk_position: i64::from(is_pk),
            writable: true,
        }
    }

    #[test]
    fn test_fits_easily() {
        let cols = vec![
            make_col("id", "INTEGER", true),
            make_col("value", "INTEGER", false),
        ];
        let rows: Vec<Vec<SqlValue>> = vec![vec![SqlValue::Integer(1), SqlValue::Integer(100)]];
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &[]);
        assert_eq!(result.len(), 2);
        assert!(result.iter().map(|&w| w as u32).sum::<u32>() <= 200);
        for &w in &result {
            assert!(w >= 6, "width {w} < 6");
        }
    }

    #[test]
    fn test_just_fits() {
        // 2 columns, each with preferred ~10; avail = 20 = sum of preferred
        let cols = vec![
            make_col("ab", "INTEGER", false),
            make_col("cd", "INTEGER", false),
        ];
        // Numeric columns can shrink to compact meta row width.
        let rows: Vec<Vec<SqlValue>> = vec![];
        let result = compute_col_widths(&cols, &rows, 12, &HashMap::new(), &[]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], 6);
        assert_eq!(result[1], 6);
    }

    #[test]
    fn test_doesnt_fit_enables_hscroll() {
        // Many columns in a narrow terminal: total_min > avail
        let cols = vec![
            make_col("ab", "INTEGER", false),
            make_col("cd", "INTEGER", false),
            make_col("ef", "INTEGER", false),
        ];
        // min_width for each = 7; total_min = 21 > 10
        let rows: Vec<Vec<SqlValue>> = vec![];
        let result = compute_col_widths(&cols, &rows, 10, &HashMap::new(), &[]);
        assert_eq!(result.len(), 3);
        // Case 3: each at min_width
        for &w in &result {
            assert!(w >= 6);
        }
        // Sum > avail_width (hscroll needed)
        let total: u32 = result.iter().map(|&w| w as u32).sum();
        assert!(total > 10, "expected total > 10, got {total}");
    }

    #[test]
    fn test_all_numeric_columns() {
        let cols = vec![
            make_col("a", "INTEGER", false),
            make_col("b", "REAL", false),
            make_col("c", "NUMERIC", false),
        ];
        let rows = vec![vec![
            SqlValue::Integer(42),
            SqlValue::Real(3.125),
            SqlValue::Integer(99),
        ]];
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &[]);
        assert_eq!(result.len(), 3);
        let total: u32 = result.iter().map(|&w| w as u32).sum();
        assert!(total <= 200);
        for &w in &result {
            assert!(w >= 6);
        }
    }

    #[test]
    fn test_all_text_columns() {
        let cols = vec![
            make_col("name", "TEXT", false),
            make_col("desc", "TEXT", false),
        ];
        let rows = vec![vec![
            SqlValue::Text("hello world".to_string()),
            SqlValue::Text("short".to_string()),
        ]];
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &[]);
        assert_eq!(result.len(), 2);
        for &w in &result {
            assert!(w >= 6);
        }
    }

    #[test]
    fn test_one_huge_text_column() {
        let cols = vec![make_col("bio", "TEXT", false)];
        let long_text = "a".repeat(500);
        let rows = vec![vec![SqlValue::Text(long_text)]];
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &[]);
        assert_eq!(result.len(), 1);
        assert!(result[0] <= 40, "width {} > 40", result[0]);
        assert!(result[0] >= 6);
    }

    #[test]
    fn test_long_header_short_content() {
        let cols = vec![make_col("very_long_column_name", "TEXT", false)];
        let rows = vec![vec![SqlValue::Text("x".to_string())]];
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &[]);
        assert_eq!(result.len(), 1);
        assert!(result[0] >= 23, "width {} < 23", result[0]);
    }

    #[test]
    fn test_trimmed_average_ignores_three_high_and_low_outliers() {
        let cols = vec![make_col("status", "TEXT", false)];
        let rows = vec![
            vec![SqlValue::Text("a".to_string())],
            vec![SqlValue::Text("bb".to_string())],
            vec![SqlValue::Text("ccc".to_string())],
            vec![SqlValue::Text("medium_len".to_string())],
            vec![SqlValue::Text("medium_len".to_string())],
            vec![SqlValue::Text("medium_len".to_string())],
            vec![SqlValue::Text("medium_len".to_string())],
            vec![SqlValue::Text("x".repeat(100))],
            vec![SqlValue::Text("x".repeat(200))],
            vec![SqlValue::Text("x".repeat(300))],
        ];
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0], 12,
            "width {} should use the trimmed average",
            result[0]
        );
    }

    #[test]
    fn test_heuristic_width_caps_at_40_chars() {
        let cols = vec![make_col("notes", "TEXT", false)];
        let rows = (0..10)
            .map(|_| vec![SqlValue::Text("x".repeat(120))])
            .collect::<Vec<_>>();
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], 40);
    }

    #[test]
    fn test_numeric_column_header_still_fits() {
        let cols = vec![make_col("very_long_integer_id", "INTEGER", false)];
        let rows = vec![vec![SqlValue::Integer(7)]];
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &[]);
        assert_eq!(result.len(), 1);
        assert!(
            result[0] >= 22,
            "width {} should fit the column header",
            result[0]
        );
    }

    #[test]
    fn test_crowded_text_columns_keep_content_based_widths() {
        let cols = vec![
            make_col("account", "TEXT", false),
            make_col("subject", "TEXT", false),
            make_col("body", "TEXT", false),
        ];
        let rows = vec![vec![
            SqlValue::Text("andreas.herdt@example.com".to_string()),
            SqlValue::Text("Invoice reminder".to_string()),
            SqlValue::Text("This message has a longer preview body".to_string()),
        ]];
        let result = compute_col_widths(&cols, &rows, 24, &HashMap::new(), &[]);
        assert_eq!(result.len(), 3);
        assert!(
            result[0] > 12,
            "account width {} still too header-driven",
            result[0]
        );
        let total: u32 = result.iter().map(|&w| w as u32).sum();
        assert!(
            total > 24,
            "expected horizontal scroll sizing, got total {total}"
        );
    }

    #[test]
    fn test_mixed_pk_fk_columns() {
        let cols = vec![
            make_col("id", "INTEGER", true),
            make_col("name", "TEXT", false),
            make_col("ref_id", "INTEGER", false),
        ];
        let fk_cols = vec![false, false, true];
        let rows = vec![vec![
            SqlValue::Integer(1),
            SqlValue::Text("Alice".to_string()),
            SqlValue::Integer(42),
        ]];
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &fk_cols);
        assert_eq!(result.len(), 3);
        assert!(result[0] >= 7, "id width {} < 7", result[0]);
        assert!(result[2] >= 10, "ref_id width {} < 10", result[2]);
    }

    #[test]
    fn test_narrow_terminal() {
        let cols = vec![
            make_col("ab", "INTEGER", false),
            make_col("cd", "TEXT", false),
        ];
        // min for "ab": 2+3+0+2=7; min for "cd": 2+3+0+2=7; total_min=14
        // avail = 20 -> 14 <= 20 < preferred -> Case 2
        let rows: Vec<Vec<SqlValue>> = vec![];
        let result = compute_col_widths(&cols, &rows, 20, &HashMap::new(), &[]);
        assert_eq!(result.len(), 2);
        for &w in &result {
            assert!(w >= 6);
        }
        let total: u32 = result.iter().map(|&w| w as u32).sum();
        assert!(total <= 20, "total {total} > 20");
    }

    #[test]
    fn test_many_columns() {
        let cols: Vec<Column> = (0..15)
            .map(|i| make_col(&format!("c{i}"), "INTEGER", false))
            .collect();
        let rows: Vec<Vec<SqlValue>> = vec![];
        let result = compute_col_widths(&cols, &rows, 300, &HashMap::new(), &[]);
        assert_eq!(result.len(), 15);
    }

    #[test]
    fn test_manual_widths() {
        let cols = vec![
            make_col("id", "INTEGER", false),
            make_col("name", "TEXT", false),
        ];
        let mut manual = HashMap::new();
        manual.insert(0usize, 15u16);
        let rows: Vec<Vec<SqlValue>> = vec![];
        let result = compute_col_widths(&cols, &rows, 100, &manual, &[]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], 15, "manual width should be exactly 15");
        assert!(result[1] >= 6);
    }

    #[test]
    fn test_empty_rows() {
        let cols = vec![
            make_col("id", "INTEGER", false),
            make_col("name", "TEXT", false),
        ];
        let rows: Vec<Vec<SqlValue>> = vec![];
        let result = compute_col_widths(&cols, &rows, 200, &HashMap::new(), &[]);
        assert_eq!(result.len(), 2);
        for &w in &result {
            assert!(w >= 6);
        }
    }

    #[test]
    fn test_zero_avail_width() {
        let cols = vec![make_col("id", "INTEGER", false)];
        let rows: Vec<Vec<SqlValue>> = vec![];
        let result = compute_col_widths(&cols, &rows, 0, &HashMap::new(), &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], 0);
    }

    #[test]
    fn test_empty_columns() {
        let cols: Vec<Column> = vec![];
        let rows: Vec<Vec<SqlValue>> = vec![];
        let result = compute_col_widths(&cols, &rows, 100, &HashMap::new(), &[]);
        assert_eq!(result.len(), 0);
    }
}
