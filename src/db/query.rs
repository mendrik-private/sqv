//! Small, shared SQL construction primitives.
//!
//! Values always remain bound parameters. SQLite does not support binding object
//! names, so identifiers are quoted here in one place.

pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::quote_identifier;

    #[test]
    fn quotes_embedded_double_quotes() {
        assert_eq!(quote_identifier("say \"hello\""), "\"say \"\"hello\"\"\"");
    }
}
