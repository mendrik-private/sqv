use super::rule::{FilterOp, FilterSet, FilterValue};
use crate::db::query::quote_identifier;
use crate::db::types::SqlValue;
use rusqlite::types::Value as RusqliteValue;

fn sql_val(v: &SqlValue) -> RusqliteValue {
    match v {
        SqlValue::Null => RusqliteValue::Null,
        SqlValue::Integer(n) => RusqliteValue::Integer(*n),
        SqlValue::Real(f) => RusqliteValue::Real(*f),
        SqlValue::Text(s) => RusqliteValue::Text(s.clone()),
        SqlValue::Blob(b) => RusqliteValue::Blob(b.clone()),
    }
}

/// Returns (WHERE clause without "WHERE", params vec).
/// The WHERE clause uses ?1, ?2, ... positional params.
pub fn filter_to_sql(filter: &FilterSet) -> anyhow::Result<(String, Vec<RusqliteValue>)> {
    let mut parts: Vec<String> = Vec::new();
    let mut params: Vec<RusqliteValue> = Vec::new();
    let mut param_idx = 1usize;

    for (col_name, col_filter) in &filter.columns {
        let enabled_rules: Vec<_> = col_filter.rules.iter().filter(|r| r.enabled).collect();
        if enabled_rules.is_empty() {
            continue;
        }

        let col_parts: Vec<String> = enabled_rules
            .iter()
            .map(|rule| -> anyhow::Result<String> {
                let col = quote_identifier(col_name);
                match &rule.op {
                    FilterOp::Eq => {
                        params.push(sql_val(expect_literal(&rule.value, &rule.op)?));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} = {}", col, p))
                    }
                    FilterOp::Ne => {
                        params.push(sql_val(expect_literal(&rule.value, &rule.op)?));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} != {}", col, p))
                    }
                    FilterOp::Lt => {
                        params.push(sql_val(expect_literal(&rule.value, &rule.op)?));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} < {}", col, p))
                    }
                    FilterOp::Le => {
                        params.push(sql_val(expect_literal(&rule.value, &rule.op)?));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} <= {}", col, p))
                    }
                    FilterOp::Gt => {
                        params.push(sql_val(expect_literal(&rule.value, &rule.op)?));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} > {}", col, p))
                    }
                    FilterOp::Ge => {
                        params.push(sql_val(expect_literal(&rule.value, &rule.op)?));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} >= {}", col, p))
                    }
                    FilterOp::Contains => {
                        let pattern = format!(
                            "%{}%",
                            escape_like_literal(expect_pattern(&rule.value, &rule.op)?)
                        );
                        params.push(RusqliteValue::Text(pattern));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} LIKE {} ESCAPE '\\'", col, p))
                    }
                    FilterOp::NotContains => {
                        let pattern = format!(
                            "%{}%",
                            escape_like_literal(expect_pattern(&rule.value, &rule.op)?)
                        );
                        params.push(RusqliteValue::Text(pattern));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} NOT LIKE {} ESCAPE '\\'", col, p))
                    }
                    FilterOp::StartsWith => {
                        let pattern = format!(
                            "{}%",
                            escape_like_literal(expect_pattern(&rule.value, &rule.op)?)
                        );
                        params.push(RusqliteValue::Text(pattern));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} LIKE {} ESCAPE '\\'", col, p))
                    }
                    FilterOp::EndsWith => {
                        let pattern = format!(
                            "%{}",
                            escape_like_literal(expect_pattern(&rule.value, &rule.op)?)
                        );
                        params.push(RusqliteValue::Text(pattern));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} LIKE {} ESCAPE '\\'", col, p))
                    }
                    FilterOp::Like => {
                        let pattern = expect_pattern(&rule.value, &rule.op)?.to_string();
                        params.push(RusqliteValue::Text(pattern));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("{} LIKE {}", col, p))
                    }
                    FilterOp::Regex => {
                        let FilterValue::Regex(pattern) = &rule.value else {
                            anyhow::bail!("regex filter requires a regular expression");
                        };
                        regex::Regex::new(pattern).map_err(|error| {
                            anyhow::anyhow!("invalid regular expression: {error}")
                        })?;
                        params.push(RusqliteValue::Text(pattern.clone()));
                        let p = format!("?{}", param_idx);
                        param_idx += 1;
                        Ok(format!("regexp({}, {})", p, col))
                    }
                    FilterOp::IsNull => Ok(format!("{} IS NULL", col)),
                    FilterOp::IsNotNull => Ok(format!("{} IS NOT NULL", col)),
                    FilterOp::Between => {
                        if let FilterValue::Range(lo, hi) = &rule.value {
                            params.push(sql_val(lo));
                            let p1 = format!("?{}", param_idx);
                            param_idx += 1;
                            params.push(sql_val(hi));
                            let p2 = format!("?{}", param_idx);
                            param_idx += 1;
                            Ok(format!("{} BETWEEN {} AND {}", col, p1, p2))
                        } else {
                            anyhow::bail!("between filter requires a lower and upper value")
                        }
                    }
                    FilterOp::In => {
                        if let FilterValue::List(vals) = &rule.value {
                            let placeholders: Vec<String> = vals
                                .iter()
                                .map(|v| {
                                    params.push(sql_val(v));
                                    let p = format!("?{}", param_idx);
                                    param_idx += 1;
                                    p
                                })
                                .collect();
                            if placeholders.is_empty() {
                                anyhow::bail!("in filter requires at least one value");
                            }
                            Ok(format!("{} IN ({})", col, placeholders.join(", ")))
                        } else {
                            anyhow::bail!("in filter requires a list of values")
                        }
                    }
                    FilterOp::Today => Ok(format!("date({}) = date('now')", col)),
                    FilterOp::ThisWeek => Ok(format!(
                        "date({}) >= date('now', 'weekday 0', '-6 days')",
                        col
                    )),
                    FilterOp::ThisMonth => Ok(format!(
                        "strftime('%Y-%m', {}) = strftime('%Y-%m', 'now')",
                        col
                    )),
                    FilterOp::ThisYear => {
                        Ok(format!("strftime('%Y', {}) = strftime('%Y', 'now')", col))
                    }
                    FilterOp::LastNDays => {
                        let n = if let FilterValue::N(n) = &rule.value {
                            *n
                        } else {
                            anyhow::bail!("last-N-days filter requires a day count")
                        };
                        if n < 0 {
                            anyhow::bail!("last-N-days filter cannot be negative");
                        }
                        Ok(format!("date({}) >= date('now', '-{} days')", col, n))
                    }
                    FilterOp::Formula => {
                        if let FilterValue::Formula(formula) = &rule.value {
                            sanitize_formula(formula, col_name)
                        } else {
                            anyhow::bail!("formula filter requires a formula")
                        }
                    }
                }
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        if col_parts.len() == 1 {
            parts.push(col_parts.into_iter().next().unwrap_or_default());
        } else {
            parts.push(format!("({})", col_parts.join(" OR ")));
        }
    }

    let where_clause = parts.join(" AND ");
    Ok((where_clause, params))
}

fn expect_literal<'a>(value: &'a FilterValue, op: &FilterOp) -> anyhow::Result<&'a SqlValue> {
    match value {
        FilterValue::Literal(value) => Ok(value),
        _ => anyhow::bail!("{op:?} filter requires a literal value"),
    }
}

fn expect_pattern<'a>(value: &'a FilterValue, op: &FilterOp) -> anyhow::Result<&'a str> {
    match value {
        FilterValue::Pattern(value) => Ok(value),
        _ => anyhow::bail!("{op:?} filter requires a text pattern"),
    }
}

fn escape_like_literal(pattern: &str) -> String {
    let mut escaped = String::with_capacity(pattern.len());
    for ch in pattern.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn sanitize_formula(formula: &str, col_name: &str) -> anyhow::Result<String> {
    let quoted_col = quote_identifier(col_name);
    let mut result = String::new();
    let mut i = 0;
    let bytes = formula.as_bytes();
    while i < bytes.len() {
        if bytes[i..].starts_with(b"col") {
            let after = i + 3;
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let after_ok = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
            if before_ok && after_ok {
                result.push_str(&quoted_col);
                i += 3;
                continue;
            }
        }
        let c = bytes[i] as char;
        match c {
            '0'..='9'
            | '+'
            | '-'
            | '*'
            | '/'
            | '('
            | ')'
            | '>'
            | '<'
            | '='
            | '!'
            | '.'
            | ' '
            | '\t' => {
                result.push(c);
            }
            _ => anyhow::bail!("formula contains an unsupported character"),
        }
        i += 1;
    }
    if result.trim().is_empty() {
        anyhow::bail!("formula cannot be empty");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::types::SqlValue;
    use crate::filter::rule::{ColumnFilter, FilterOp, FilterRule, FilterSet, FilterValue};

    fn make_set(col: &str, op: FilterOp, val: FilterValue) -> FilterSet {
        let mut fs = FilterSet::default();
        fs.columns.insert(
            col.to_string(),
            ColumnFilter {
                rules: vec![FilterRule {
                    op,
                    value: val,
                    enabled: true,
                    label: None,
                }],
            },
        );
        fs
    }

    #[test]
    fn test_simple_equality() {
        let fs = make_set(
            "id",
            FilterOp::Eq,
            FilterValue::Literal(SqlValue::Integer(42)),
        );
        let (clause, params) = filter_to_sql(&fs).expect("valid filter");
        assert!(clause.contains("\"id\" = ?1"), "got: {}", clause);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], RusqliteValue::Integer(42));
    }

    #[test]
    fn test_text_contains() {
        let fs = make_set(
            "name",
            FilterOp::Contains,
            FilterValue::Pattern("foo".to_string()),
        );
        let (clause, params) = filter_to_sql(&fs).expect("valid filter");
        assert!(clause.contains("LIKE"), "got: {}", clause);
        match params.first() {
            Some(RusqliteValue::Text(p)) => assert_eq!(p, "%foo%"),
            _ => panic!("expected Text param"),
        }
    }

    #[test]
    fn test_between() {
        let fs = make_set(
            "age",
            FilterOp::Between,
            FilterValue::Range(SqlValue::Integer(18), SqlValue::Integer(65)),
        );
        let (clause, params) = filter_to_sql(&fs).expect("valid filter");
        assert!(clause.contains("BETWEEN"), "got: {}", clause);
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_or_within_column() {
        let mut fs = FilterSet::default();
        fs.columns.insert(
            "status".to_string(),
            ColumnFilter {
                rules: vec![
                    FilterRule {
                        op: FilterOp::Eq,
                        value: FilterValue::Literal(SqlValue::Text("active".to_string())),
                        enabled: true,
                        label: None,
                    },
                    FilterRule {
                        op: FilterOp::Eq,
                        value: FilterValue::Literal(SqlValue::Text("pending".to_string())),
                        enabled: true,
                        label: None,
                    },
                ],
            },
        );
        let (clause, _) = filter_to_sql(&fs).expect("valid filter");
        assert!(clause.contains(" OR "), "got: {}", clause);
    }

    #[test]
    fn test_and_across_columns() {
        let mut fs = FilterSet::default();
        fs.columns.insert(
            "age".to_string(),
            ColumnFilter {
                rules: vec![FilterRule {
                    op: FilterOp::Gt,
                    value: FilterValue::Literal(SqlValue::Integer(18)),
                    enabled: true,
                    label: None,
                }],
            },
        );
        fs.columns.insert(
            "name".to_string(),
            ColumnFilter {
                rules: vec![FilterRule {
                    op: FilterOp::Contains,
                    value: FilterValue::Pattern("foo".to_string()),
                    enabled: true,
                    label: None,
                }],
            },
        );
        let (clause, _) = filter_to_sql(&fs).expect("valid filter");
        assert!(
            clause.contains("\"age\"") && clause.contains("\"name\""),
            "got: {}",
            clause
        );
    }

    #[test]
    fn test_null_checks() {
        let fs_null = make_set(
            "email",
            FilterOp::IsNull,
            FilterValue::Literal(SqlValue::Null),
        );
        let (clause, params) = filter_to_sql(&fs_null).expect("valid filter");
        assert!(clause.contains("IS NULL"), "got: {}", clause);
        assert_eq!(params.len(), 0);

        let fs_notnull = make_set(
            "email",
            FilterOp::IsNotNull,
            FilterValue::Literal(SqlValue::Null),
        );
        let (clause2, _) = filter_to_sql(&fs_notnull).expect("valid filter");
        assert!(clause2.contains("IS NOT NULL"), "got: {}", clause2);
    }

    #[test]
    fn test_disabled_rule_excluded() {
        let mut fs = FilterSet::default();
        fs.columns.insert(
            "id".to_string(),
            ColumnFilter {
                rules: vec![FilterRule {
                    op: FilterOp::Eq,
                    value: FilterValue::Literal(SqlValue::Integer(42)),
                    enabled: false,
                    label: None,
                }],
            },
        );
        let (clause, params) = filter_to_sql(&fs).expect("valid filter");
        assert!(clause.is_empty(), "got: {}", clause);
        assert_eq!(params.len(), 0);
    }

    #[test]
    fn test_empty_filter_set() {
        let fs = FilterSet::default();
        let (clause, params) = filter_to_sql(&fs).expect("valid filter");
        assert!(clause.is_empty());
        assert!(params.is_empty());
    }

    #[test]
    fn test_starts_with() {
        let fs = make_set(
            "title",
            FilterOp::StartsWith,
            FilterValue::Pattern("foo".to_string()),
        );
        let (clause, params) = filter_to_sql(&fs).expect("valid filter");
        assert!(clause.contains("LIKE"), "got: {}", clause);
        match params.first() {
            Some(RusqliteValue::Text(p)) => assert_eq!(p, "foo%"),
            _ => panic!("expected Text param"),
        }
    }

    #[test]
    fn literal_text_operators_escape_like_wildcards_and_escape_character() {
        let cases = [
            (
                FilterOp::Contains,
                "%50\\%\\_\\\\off%",
                "\"name\" LIKE ?1 ESCAPE '\\'",
            ),
            (
                FilterOp::NotContains,
                "%50\\%\\_\\\\off%",
                "\"name\" NOT LIKE ?1 ESCAPE '\\'",
            ),
            (
                FilterOp::StartsWith,
                "50\\%\\_\\\\off%",
                "\"name\" LIKE ?1 ESCAPE '\\'",
            ),
            (
                FilterOp::EndsWith,
                "%50\\%\\_\\\\off",
                "\"name\" LIKE ?1 ESCAPE '\\'",
            ),
        ];

        for (op, expected_pattern, expected_clause) in cases {
            let filter = make_set("name", op, FilterValue::Pattern("50%_\\off".to_string()));
            let (clause, params) = filter_to_sql(&filter).expect("valid literal text filter");

            assert_eq!(clause, expected_clause);
            assert_eq!(
                params,
                vec![RusqliteValue::Text(expected_pattern.to_string())]
            );
        }
    }

    #[test]
    fn explicit_like_preserves_wildcard_pattern() {
        let pattern = "50%_\\off";
        let filter = make_set(
            "name",
            FilterOp::Like,
            FilterValue::Pattern(pattern.to_string()),
        );
        let (clause, params) = filter_to_sql(&filter).expect("valid LIKE filter");

        assert_eq!(clause, "\"name\" LIKE ?1");
        assert_eq!(params, vec![RusqliteValue::Text(pattern.to_string())]);
    }

    #[test]
    fn rejects_rule_value_mismatches() {
        let filter = make_set(
            "name",
            FilterOp::Eq,
            FilterValue::Pattern("unexpected".to_string()),
        );
        assert!(filter_to_sql(&filter).is_err());
    }

    #[test]
    fn rejects_invalid_regex_and_formula() {
        let regex = make_set("name", FilterOp::Regex, FilterValue::Regex("[".to_string()));
        assert!(filter_to_sql(&regex).is_err());

        let formula = make_set(
            "amount",
            FilterOp::Formula,
            FilterValue::Formula("col; DROP TABLE items".to_string()),
        );
        assert!(filter_to_sql(&formula).is_err());
    }
}
