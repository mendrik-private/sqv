use std::borrow::Cow;

use rusqlite::types::{ToSqlOutput, Value, ValueRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl SqlValue {
    /// The value as plain text, with NULL spelled out and BLOBs summarized by size.
    pub fn to_text(&self) -> Cow<'_, str> {
        match self {
            SqlValue::Null => Cow::Borrowed("NULL"),
            SqlValue::Integer(n) => Cow::Owned(n.to_string()),
            SqlValue::Real(f) => Cow::Owned(f.to_string()),
            SqlValue::Text(s) => Cow::Borrowed(s),
            SqlValue::Blob(bytes) => Cow::Owned(format!("<blob {} bytes>", bytes.len())),
        }
    }
}

impl From<ValueRef<'_>> for SqlValue {
    fn from(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Null => SqlValue::Null,
            ValueRef::Integer(n) => SqlValue::Integer(n),
            ValueRef::Real(f) => SqlValue::Real(f),
            ValueRef::Text(bytes) => SqlValue::Text(String::from_utf8_lossy(bytes).into_owned()),
            ValueRef::Blob(bytes) => SqlValue::Blob(bytes.to_vec()),
        }
    }
}

impl From<&SqlValue> for Value {
    fn from(value: &SqlValue) -> Self {
        match value {
            SqlValue::Null => Value::Null,
            SqlValue::Integer(n) => Value::Integer(*n),
            SqlValue::Real(f) => Value::Real(*f),
            SqlValue::Text(s) => Value::Text(s.clone()),
            SqlValue::Blob(bytes) => Value::Blob(bytes.clone()),
        }
    }
}

impl rusqlite::ToSql for SqlValue {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self {
            SqlValue::Null => ValueRef::Null,
            SqlValue::Integer(n) => ValueRef::Integer(*n),
            SqlValue::Real(f) => ValueRef::Real(*f),
            SqlValue::Text(s) => ValueRef::Text(s.as_bytes()),
            SqlValue::Blob(bytes) => ValueRef::Blob(bytes),
        }))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColAffinity {
    Integer,
    Real,
    Numeric,
    Text,
    Blob,
}

/// Whether the column only accepts numbers from the editors.
pub fn expects_number(col_type: &str) -> bool {
    matches!(affinity(col_type), ColAffinity::Integer | ColAffinity::Real)
}

/// Converts text entered for a column into the value to store. INTEGER and REAL
/// columns reject text that is not a number, so typos surface before writing.
/// NUMERIC columns follow SQLite: numbers are stored as numbers, anything else
/// (such as a date) stays text. TEXT and BLOB columns store the text unchanged.
pub fn parse_input(col_type: &str, text: &str) -> Result<SqlValue, &'static str> {
    match affinity(col_type) {
        ColAffinity::Integer => text
            .parse()
            .map(SqlValue::Integer)
            .map_err(|_| "must be an integer"),
        ColAffinity::Real => text
            .parse()
            .map(SqlValue::Real)
            .map_err(|_| "must be a number"),
        ColAffinity::Numeric => Ok(text
            .parse()
            .map(SqlValue::Integer)
            .or_else(|_| text.parse().map(SqlValue::Real))
            .unwrap_or_else(|_| SqlValue::Text(text.to_string()))),
        ColAffinity::Text | ColAffinity::Blob => Ok(SqlValue::Text(text.to_string())),
    }
}

pub fn affinity(col_type: &str) -> ColAffinity {
    let upper = col_type.to_uppercase();
    if upper.contains("INT") {
        return ColAffinity::Integer;
    }
    if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        return ColAffinity::Text;
    }
    if upper.is_empty() || upper.contains("BLOB") {
        return ColAffinity::Blob;
    }
    if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        return ColAffinity::Real;
    }
    ColAffinity::Numeric
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalKind {
    Date,
    Datetime,
}

/// Date or datetime semantics implied by a declared column type. SQLite has no
/// temporal storage class, so this is a naming convention, not an affinity.
pub fn temporal_kind(col_type: &str) -> Option<TemporalKind> {
    let upper = col_type.to_uppercase();
    if upper.contains("TIMESTAMP")
        || upper.contains("DATETIME")
        || (upper.contains("DATE") && upper.contains("TIME"))
    {
        Some(TemporalKind::Datetime)
    } else if upper.contains("DATE") {
        Some(TemporalKind::Date)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_input, SqlValue};

    #[test]
    fn input_parsing_follows_column_affinity() {
        assert_eq!(parse_input("INTEGER", "42"), Ok(SqlValue::Integer(42)));
        assert!(parse_input("INTEGER", "4.2").is_err());
        assert_eq!(parse_input("DOUBLE", "4"), Ok(SqlValue::Real(4.0)));
        assert!(parse_input("REAL", "x").is_err());
        assert_eq!(parse_input("DECIMAL(10,2)", "7"), Ok(SqlValue::Integer(7)));
        assert_eq!(parse_input("NUMERIC", "7.5"), Ok(SqlValue::Real(7.5)));
        assert_eq!(
            parse_input("DATE", "2024-01-31"),
            Ok(SqlValue::Text("2024-01-31".into()))
        );
        assert_eq!(parse_input("TEXT", "42"), Ok(SqlValue::Text("42".into())));
    }
}
