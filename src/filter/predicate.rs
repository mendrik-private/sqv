use super::rule::{Condition, FilterSet};
use crate::db::query::quote_identifier;
use rusqlite::types::Value as RusqliteValue;

/// Returns (WHERE clause without "WHERE", params vec).
/// The WHERE clause uses ?1, ?2, ... positional params.
pub fn filter_to_sql(filter: &FilterSet) -> anyhow::Result<(String, Vec<RusqliteValue>)> {
    let mut parts: Vec<String> = Vec::new();
    let mut params: Vec<RusqliteValue> = Vec::new();

    for (col_name, col_filter) in &filter.columns {
        let col = quote_identifier(col_name);
        let mut col_parts = Vec::new();
        for rule in col_filter.rules.iter().filter(|rule| rule.enabled) {
            let mut bind = |value: RusqliteValue| {
                params.push(value);
                format!("?{}", params.len())
            };
            let part = match &rule.condition {
                Condition::Lt(value) => format!("{col} < {}", bind(value.into())),
                Condition::Gt(value) => format!("{col} > {}", bind(value.into())),
                Condition::Eq(value) => format!("{col} = {}", bind(value.into())),
                Condition::Contains(text) => {
                    let p = bind(RusqliteValue::Text(format!(
                        "%{}%",
                        escape_like_literal(text)
                    )));
                    format!("{col} LIKE {p} ESCAPE '\\'")
                }
                Condition::Regex(pattern) => {
                    regex::Regex::new(pattern)
                        .map_err(|error| anyhow::anyhow!("invalid regular expression: {error}"))?;
                    let p = bind(RusqliteValue::Text(pattern.clone()));
                    format!("regexp({p}, {col})")
                }
            };
            col_parts.push(part);
        }

        match col_parts.len() {
            0 => {}
            1 => parts.extend(col_parts),
            _ => parts.push(format!("({})", col_parts.join(" OR "))),
        }
    }

    Ok((parts.join(" AND "), params))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::types::SqlValue;
    use crate::filter::rule::{ColumnFilter, FilterRule};

    fn make_set(col: &str, conditions: Vec<Condition>) -> FilterSet {
        let mut fs = FilterSet::default();
        fs.columns.insert(
            col.to_string(),
            ColumnFilter {
                rules: conditions.into_iter().map(FilterRule::new).collect(),
            },
        );
        fs
    }

    #[test]
    fn comparisons_bind_their_literal() {
        for (condition, operator) in [
            (Condition::Eq(SqlValue::Integer(42)), "="),
            (Condition::Lt(SqlValue::Integer(42)), "<"),
            (Condition::Gt(SqlValue::Integer(42)), ">"),
        ] {
            let (clause, params) =
                filter_to_sql(&make_set("id", vec![condition])).expect("valid filter");
            assert_eq!(clause, format!("\"id\" {operator} ?1"));
            assert_eq!(params, vec![RusqliteValue::Integer(42)]);
        }
    }

    #[test]
    fn contains_escapes_like_wildcards_and_escape_character() {
        let filter = make_set("name", vec![Condition::Contains("50%_\\off".to_string())]);
        let (clause, params) = filter_to_sql(&filter).expect("valid filter");

        assert_eq!(clause, "\"name\" LIKE ?1 ESCAPE '\\'");
        assert_eq!(
            params,
            vec![RusqliteValue::Text("%50\\%\\_\\\\off%".to_string())]
        );
    }

    #[test]
    fn rules_on_one_column_are_alternatives() {
        let filter = make_set(
            "status",
            vec![
                Condition::Eq(SqlValue::Text("active".to_string())),
                Condition::Eq(SqlValue::Text("pending".to_string())),
            ],
        );
        let (clause, params) = filter_to_sql(&filter).expect("valid filter");
        assert!(clause.contains(" OR "), "got: {clause}");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn columns_are_combined_with_and() {
        let mut filter = make_set("age", vec![Condition::Gt(SqlValue::Integer(18))]);
        filter
            .columns
            .extend(make_set("name", vec![Condition::Contains("foo".to_string())]).columns);
        let (clause, _) = filter_to_sql(&filter).expect("valid filter");
        assert!(clause.contains(" AND "), "got: {clause}");
        assert!(clause.contains("\"age\"") && clause.contains("\"name\""));
    }

    #[test]
    fn disabled_rules_and_empty_sets_add_no_predicate() {
        let mut filter = make_set("id", vec![Condition::Eq(SqlValue::Integer(42))]);
        filter.columns.get_mut("id").expect("column").rules[0].enabled = false;
        for filter in [filter, FilterSet::default()] {
            let (clause, params) = filter_to_sql(&filter).expect("valid filter");
            assert!(clause.is_empty(), "got: {clause}");
            assert!(params.is_empty());
        }
    }

    #[test]
    fn rejects_invalid_regex() {
        let filter = make_set("name", vec![Condition::Regex("[".to_string())]);
        assert!(filter_to_sql(&filter).is_err());
    }
}
