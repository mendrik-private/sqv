//! Shared SQL construction primitives.
//!
//! Values always remain bound parameters. SQLite does not support binding object
//! names, so identifiers are quoted here in one place.

use rusqlite::types::Value;

use super::schema::RowIdentity;

pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderBy {
    pub column: String,
    pub ascending: bool,
}

/// A table as the user currently sees it: an optional sort column and a compiled
/// filter predicate. Row offsets are only meaningful relative to one view.
#[derive(Debug, Clone, Default)]
pub struct ViewQuery {
    pub table: String,
    pub order_by: Option<OrderBy>,
    /// WHERE predicate without the keyword, using `?1..?N` for `where_params`.
    pub where_clause: String,
    pub where_params: Vec<Value>,
}

impl ViewQuery {
    /// The whole table in row-identity order.
    pub fn table(name: &str) -> Self {
        Self {
            table: name.to_string(),
            ..Self::default()
        }
    }

    pub(crate) fn quoted_table(&self) -> String {
        quote_identifier(&self.table)
    }

    pub(crate) fn where_part(&self) -> String {
        if self.where_clause.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.where_clause)
        }
    }

    /// The user's sort followed by the row identity, which makes every offset
    /// deterministic even when sort values repeat.
    pub(crate) fn order_terms(&self, identity: &RowIdentity) -> String {
        let identity_terms = match identity {
            RowIdentity::RowidAlias(alias) => quote_identifier(alias),
            RowIdentity::PrimaryKey(columns) => columns
                .iter()
                .map(|column| format!("{} ASC", quote_identifier(column)))
                .collect::<Vec<_>>()
                .join(", "),
        };
        match &self.order_by {
            Some(order) => format!(
                "{} {}, {}",
                quote_identifier(&order.column),
                if order.ascending { "ASC" } else { "DESC" },
                identity_terms
            ),
            None => identity_terms,
        }
    }

    /// Parameters for the view predicate followed by `extra`, together with the
    /// placeholder number of the first extra parameter.
    pub(crate) fn params_with(
        &self,
        extra: impl IntoIterator<Item = Value>,
    ) -> (Vec<Value>, usize) {
        let first = self.where_params.len() + 1;
        let mut params = self.where_params.clone();
        params.extend(extra);
        (params, first)
    }
}

#[cfg(test)]
mod tests {
    use super::quote_identifier;

    #[test]
    fn quotes_embedded_double_quotes() {
        assert_eq!(quote_identifier("say \"hello\""), "\"say \"\"hello\"\"\"");
    }
}
