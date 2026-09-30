use rusqlite::{Connection, OptionalExtension};

use crate::db::schema::Column;
use crate::db::types::SqlValue;
use crate::db::{decode_row_values, mutation_rowid_alias, query::quote_identifier};

// Every mutation runs in a transaction that rolls back when dropped uncommitted,
// so early returns through `?` never leave partial writes behind.

fn expect_one_row(affected: usize, rowid: i64) -> anyhow::Result<()> {
    match affected {
        1 => Ok(()),
        0 => anyhow::bail!("row {} no longer exists", rowid),
        count => anyhow::bail!("row identity matched {} rows", count),
    }
}

pub fn commit_cell_edit(
    conn: &Connection,
    table: &str,
    col: &str,
    rowid: i64,
    value: &SqlValue,
) -> anyhow::Result<()> {
    let rowid_alias = mutation_rowid_alias(conn, table)?;
    let tx = conn.unchecked_transaction()?;
    let query = format!(
        "UPDATE {} SET {} = ?1 WHERE {} = ?2",
        quote_identifier(table),
        quote_identifier(col),
        quote_identifier(&rowid_alias)
    );
    expect_one_row(tx.execute(&query, rusqlite::params![value, rowid])?, rowid)?;
    tx.commit()?;
    Ok(())
}

pub fn insert_row(
    conn: &Connection,
    table: &str,
    values: &[(String, SqlValue)],
) -> anyhow::Result<i64> {
    mutation_rowid_alias(conn, table)?;
    let tx = conn.unchecked_transaction()?;
    if values.is_empty() {
        tx.execute(
            &format!("INSERT INTO {} DEFAULT VALUES", quote_identifier(table)),
            [],
        )?;
    } else {
        let col_names = values
            .iter()
            .map(|(name, _)| quote_identifier(name))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=values.len())
            .map(|idx| format!("?{}", idx))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_identifier(table),
            col_names,
            placeholders
        );
        tx.execute(
            &sql,
            rusqlite::params_from_iter(values.iter().map(|(_, value)| value)),
        )?;
    }
    let rowid = tx.last_insert_rowid();
    tx.commit()?;
    Ok(rowid)
}

pub fn delete_row(conn: &Connection, table: &str, rowid: i64) -> anyhow::Result<()> {
    delete_rows_by_rowids(conn, table, &[rowid]).map(drop)
}

pub fn delete_row_with_backup(
    conn: &Connection,
    table: &str,
    columns: &[Column],
    rowid: i64,
) -> anyhow::Result<Vec<(String, SqlValue)>> {
    let rowid_alias = mutation_rowid_alias(conn, table)?;
    let tx = conn.unchecked_transaction()?;
    let writable_columns = columns
        .iter()
        .filter(|column| column.writable)
        .collect::<Vec<_>>();
    let column_list = writable_columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let select_sql = format!(
        "SELECT {} FROM {} WHERE {} = ?1 LIMIT 1",
        column_list,
        quote_identifier(table),
        quote_identifier(&rowid_alias)
    );
    let values = tx
        .query_row(&select_sql, [rowid], |row| {
            decode_row_values(row, writable_columns.len())
        })
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("row {} no longer exists", rowid))?;
    let delete_sql = format!(
        "DELETE FROM {} WHERE {} = ?1",
        quote_identifier(table),
        quote_identifier(&rowid_alias)
    );
    expect_one_row(tx.execute(&delete_sql, [rowid])?, rowid)?;
    tx.commit()?;
    Ok(writable_columns
        .iter()
        .map(|column| column.name.clone())
        .zip(values)
        .collect())
}

pub fn delete_rows_by_rowids(
    conn: &Connection,
    table: &str,
    rowids: &[i64],
) -> anyhow::Result<usize> {
    if rowids.is_empty() {
        return Ok(0);
    }
    let rowid_alias = mutation_rowid_alias(conn, table)?;
    let tx = conn.unchecked_transaction()?;
    let sql = format!(
        "DELETE FROM {} WHERE {} = ?1",
        quote_identifier(table),
        quote_identifier(&rowid_alias)
    );
    let mut statement = tx.prepare(&sql)?;
    for rowid in rowids {
        expect_one_row(statement.execute([rowid])?, *rowid)?;
    }
    drop(statement);
    tx.commit()?;
    Ok(rowids.len())
}

/// Deletes every row of `table` except those with the rowids in `keep`.
pub fn clear_table(conn: &Connection, table: &str, keep: &[i64]) -> anyhow::Result<usize> {
    let mut sql = format!("DELETE FROM {}", quote_identifier(table));
    if !keep.is_empty() {
        let placeholders = (1..=keep.len())
            .map(|idx| format!("?{idx}"))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(
            " WHERE {} NOT IN ({placeholders})",
            quote_identifier(&mutation_rowid_alias(conn, table)?)
        ));
    }
    let tx = conn.unchecked_transaction()?;
    let deleted = tx.execute(&sql, rusqlite::params_from_iter(keep))?;
    tx.commit()?;
    Ok(deleted)
}

pub fn reinsert_row(
    conn: &Connection,
    table: &str,
    rowid: i64,
    cols: &[(String, SqlValue)],
) -> anyhow::Result<()> {
    if cols.is_empty() {
        anyhow::bail!("cannot restore a row without a backup payload");
    }
    let rowid_alias = mutation_rowid_alias(conn, table)?;
    let tx = conn.unchecked_transaction()?;
    let col_names = cols
        .iter()
        .map(|(n, _)| quote_identifier(n))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (2..=cols.len() + 1)
        .map(|i| format!("?{}", i))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT INTO {} ({}, {}) VALUES (?1, {})",
        quote_identifier(table),
        quote_identifier(&rowid_alias),
        col_names,
        placeholders
    );
    let rowid = SqlValue::Integer(rowid);
    tx.execute(
        &sql,
        rusqlite::params_from_iter(std::iter::once(&rowid).chain(cols.iter().map(|(_, v)| v))),
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        clear_table, commit_cell_edit, delete_row, delete_row_with_backup, insert_row, reinsert_row,
    };
    use crate::db::types::SqlValue;
    use rusqlite::Connection;

    #[test]
    fn insert_row_applies_defaults_when_only_required_values_are_provided() {
        let conn = Connection::open_in_memory().expect("open db");
        conn.execute_batch(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                age INTEGER DEFAULT 18
            );",
        )
        .expect("create table");

        let rowid = insert_row(
            &conn,
            "users",
            &[("name".to_string(), SqlValue::Text("Alice".to_string()))],
        )
        .expect("insert row");

        let (name, age): (String, i64) = conn
            .query_row(
                "SELECT name, age FROM users WHERE rowid = ?1",
                rusqlite::params![rowid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("load inserted row");

        assert_eq!(name, "Alice");
        assert_eq!(age, 18);
    }

    #[test]
    fn insert_rejects_tables_without_an_undoable_rowid() {
        let conn = Connection::open_in_memory().expect("open db");
        conn.execute_batch(
            "CREATE TABLE settings (
                namespace TEXT PRIMARY KEY,
                value TEXT NOT NULL
            ) WITHOUT ROWID;",
        )
        .expect("create table");

        let error = insert_row(
            &conn,
            "settings",
            &[
                ("namespace".to_string(), SqlValue::Text("ui".to_string())),
                ("value".to_string(), SqlValue::Text("compact".to_string())),
            ],
        )
        .expect_err("insert must fail before mutation");

        assert!(error.to_string().contains("safe rowid"));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
            .expect("count rows");
        assert_eq!(count, 0);
    }

    #[test]
    fn deleted_rows_are_removed_by_captured_identity() {
        let conn = Connection::open_in_memory().expect("open db");
        conn.execute_batch(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL
            );
            INSERT INTO users (id, name) VALUES
                (1, 'carol'),
                (2, 'alice'),
                (3, 'bravo'),
                (4, 'delta');",
        )
        .expect("seed data");

        let deleted =
            super::delete_rows_by_rowids(&conn, "users", &[3, 1]).expect("delete selected rows");

        let remaining: Vec<String> = conn
            .prepare("SELECT name FROM users ORDER BY name ASC")
            .expect("prepare remaining")
            .query_map([], |row| row.get(0))
            .expect("query remaining")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect remaining");

        assert_eq!(deleted, 2);
        assert_eq!(remaining, vec!["alice".to_string(), "delta".to_string()]);
    }

    #[test]
    fn clear_table_deletes_every_row_in_one_call() {
        let conn = Connection::open_in_memory().expect("open db");
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);
            INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob');",
        )
        .expect("seed data");

        let deleted = clear_table(&conn, "users", &[]).expect("clear table");
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .expect("count rows");

        assert_eq!(deleted, 2);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn clear_table_keeps_the_listed_rows() {
        let conn = Connection::open_in_memory().expect("open db");
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);
            INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Cy');",
        )
        .expect("seed data");

        let deleted = clear_table(&conn, "users", &[1, 3]).expect("clear table");
        let remaining = conn
            .prepare("SELECT name FROM users ORDER BY id")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .expect("remaining rows");

        assert_eq!(deleted, 1);
        assert_eq!(remaining, vec!["Alice".to_string(), "Cy".to_string()]);
    }

    #[test]
    fn declared_rowid_column_cannot_redirect_mutations() {
        let conn = Connection::open_in_memory().expect("open db");
        conn.execute_batch(
            "CREATE TABLE items (rowid INTEGER, value TEXT);
             INSERT INTO items (rowid, value) VALUES (7, 'first'), (7, 'second');",
        )
        .expect("seed data");
        let first_physical_rowid: i64 = conn
            .query_row(
                "SELECT _rowid_ FROM items ORDER BY _rowid_ LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("physical rowid");

        commit_cell_edit(
            &conn,
            "items",
            "value",
            first_physical_rowid,
            &SqlValue::Text("changed".to_string()),
        )
        .expect("update one physical row");
        delete_row(&conn, "items", first_physical_rowid).expect("delete one physical row");

        let remaining: Vec<(i64, String)> = conn
            .prepare("SELECT rowid, value FROM items")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect");
        assert_eq!(remaining, vec![(7, "second".to_string())]);
    }

    #[test]
    fn restoring_deleted_row_never_replaces_a_conflicting_row() {
        let conn = Connection::open_in_memory().expect("open db");
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER UNIQUE, value TEXT);
             INSERT INTO items (id, value) VALUES (1, 'first');",
        )
        .expect("seed data");
        let columns = crate::db::load_schema(&conn).expect("schema").tables[0]
            .columns
            .clone();
        let backup = delete_row_with_backup(&conn, "items", &columns, 1).expect("delete");
        conn.execute("INSERT INTO items (id, value) VALUES (1, 'intruder')", [])
            .expect("insert conflict");
        assert!(reinsert_row(&conn, "items", 1, &backup).is_err());
        assert_eq!(
            conn.query_row("SELECT value FROM items", [], |row| row.get::<_, String>(0))
                .expect("restored value"),
            "intruder"
        );
    }
}
