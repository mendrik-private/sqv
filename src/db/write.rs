use rusqlite::Connection;

use crate::db::schema::Column;
use crate::db::types::SqlValue;

pub fn commit_cell_edit(
    conn: &Connection,
    table: &str,
    col: &str,
    rowid: i64,
    value: &SqlValue,
) -> anyhow::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let query = format!("UPDATE \"{}\" SET \"{}\" = ?1 WHERE rowid = ?2", table, col);
    let result = match value {
        SqlValue::Null => tx.execute(&query, rusqlite::params![rusqlite::types::Null, rowid]),
        SqlValue::Integer(n) => tx.execute(&query, rusqlite::params![n, rowid]),
        SqlValue::Real(f) => tx.execute(&query, rusqlite::params![f, rowid]),
        SqlValue::Text(s) => tx.execute(&query, rusqlite::params![s, rowid]),
        SqlValue::Blob(b) => tx.execute(&query, rusqlite::params![b, rowid]),
    };
    match result {
        Ok(1) => {
            tx.commit()?;
            Ok(())
        }
        Ok(0) => {
            let _ = tx.rollback();
            Err(anyhow::anyhow!("no row matched rowid {}", rowid))
        }
        Ok(n) => {
            let _ = tx.rollback();
            Err(anyhow::anyhow!("unexpected update count: {}", n))
        }
        Err(e) => {
            let _ = tx.rollback();
            Err(anyhow::anyhow!("{}", e))
        }
    }
}

pub fn insert_row(
    conn: &Connection,
    table: &str,
    values: &[(String, SqlValue)],
) -> anyhow::Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let result = if values.is_empty() {
        tx.execute(&format!("INSERT INTO \"{}\" DEFAULT VALUES", table), [])
    } else {
        let col_names = values
            .iter()
            .map(|(name, _)| format!("\"{}\"", name))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=values.len())
            .map(|idx| format!("?{}", idx))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO \"{}\" ({}) VALUES ({})",
            table, col_names, placeholders
        );
        let params: Vec<rusqlite::types::Value> = values
            .iter()
            .map(|(_, value)| match value {
                SqlValue::Null => rusqlite::types::Value::Null,
                SqlValue::Integer(n) => rusqlite::types::Value::Integer(*n),
                SqlValue::Real(f) => rusqlite::types::Value::Real(*f),
                SqlValue::Text(s) => rusqlite::types::Value::Text(s.clone()),
                SqlValue::Blob(bytes) => rusqlite::types::Value::Blob(bytes.clone()),
            })
            .collect();
        tx.execute(&sql, rusqlite::params_from_iter(params.iter()))
    };
    match result {
        Ok(_) => {
            let rowid = tx.last_insert_rowid();
            tx.commit()?;
            Ok(rowid)
        }
        Err(e) => {
            let _ = tx.rollback();
            Err(anyhow::anyhow!("{}", e))
        }
    }
}

pub fn delete_row(conn: &Connection, table: &str, rowid: i64) -> anyhow::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let result = tx.execute(
        &format!("DELETE FROM \"{}\" WHERE rowid = ?1", table),
        rusqlite::params![rowid],
    );
    match result {
        Ok(_) => {
            tx.commit()?;
            Ok(())
        }
        Err(e) => {
            let _ = tx.rollback();
            Err(anyhow::anyhow!("{}", e))
        }
    }
}

fn delete_order_terms(order_by: Option<(&str, bool)>) -> String {
    match order_by {
        Some((col, asc)) => format!(
            "\"{}\" {}, rowid ASC",
            col,
            if asc { "ASC" } else { "DESC" }
        ),
        None => "rowid ASC".to_string(),
    }
}

fn delete_where_part(where_clause: &str) -> String {
    if where_clause.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clause)
    }
}

pub fn delete_rows_in_view(
    conn: &Connection,
    table: &str,
    start_offset: i64,
    end_offset: i64,
    order_by: Option<(&str, bool)>,
    where_clause: &str,
    where_params: &[rusqlite::types::Value],
) -> anyhow::Result<usize> {
    if end_offset < start_offset {
        return Ok(0);
    }

    let tx = conn.unchecked_transaction()?;
    let count = end_offset - start_offset + 1;
    let where_part = delete_where_part(where_clause);
    let order_terms = delete_order_terms(order_by);
    let limit_param = where_params.len() + 1;
    let offset_param = where_params.len() + 2;
    let sql = format!(
        "DELETE FROM \"{table}\" WHERE rowid IN (
            SELECT rowid FROM \"{table}\"{where_part}
            ORDER BY {order_terms}
            LIMIT ?{limit_param} OFFSET ?{offset_param}
        )"
    );
    let mut params = where_params.to_vec();
    params.push(rusqlite::types::Value::Integer(count));
    params.push(rusqlite::types::Value::Integer(start_offset));

    match tx.execute(&sql, rusqlite::params_from_iter(params.iter())) {
        Ok(deleted) => {
            tx.commit()?;
            Ok(deleted)
        }
        Err(err) => {
            let _ = tx.rollback();
            Err(anyhow::anyhow!("{}", err))
        }
    }
}

pub fn clear_table(conn: &Connection, table: &str) -> anyhow::Result<usize> {
    let tx = conn.unchecked_transaction()?;
    match tx.execute(&format!("DELETE FROM \"{}\"", table), []) {
        Ok(deleted) => {
            tx.commit()?;
            Ok(deleted)
        }
        Err(err) => {
            let _ = tx.rollback();
            Err(anyhow::anyhow!("{}", err))
        }
    }
}

#[allow(dead_code)]
pub fn fetch_row_by_rowid(
    conn: &Connection,
    table: &str,
    columns: &[Column],
    rowid: i64,
) -> anyhow::Result<Option<Vec<SqlValue>>> {
    let col_list: String = columns
        .iter()
        .map(|c| format!("\"{}\"", c.name))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {} FROM \"{}\" WHERE rowid = ?1 LIMIT 1",
        col_list, table
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map(rusqlite::params![rowid], |row| {
        let mut vals = Vec::new();
        for i in 0..columns.len() {
            let v = match row.get_ref(i)? {
                rusqlite::types::ValueRef::Null => SqlValue::Null,
                rusqlite::types::ValueRef::Integer(n) => SqlValue::Integer(n),
                rusqlite::types::ValueRef::Real(f) => SqlValue::Real(f),
                rusqlite::types::ValueRef::Text(b) => {
                    SqlValue::Text(String::from_utf8_lossy(b).into_owned())
                }
                rusqlite::types::ValueRef::Blob(b) => SqlValue::Blob(b.to_vec()),
            };
            vals.push(v);
        }
        Ok(vals)
    })?;
    Ok(rows.next().transpose()?)
}

pub fn reinsert_row(
    conn: &Connection,
    table: &str,
    rowid: i64,
    cols: &[(String, SqlValue)],
) -> anyhow::Result<()> {
    if cols.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    let col_names = cols
        .iter()
        .map(|(n, _)| format!("\"{}\"", n))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (2..=cols.len() + 1)
        .map(|i| format!("?{}", i))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT OR REPLACE INTO \"{}\" (rowid, {}) VALUES (?1, {})",
        table, col_names, placeholders
    );
    let mut all_params: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Integer(rowid)];
    for (_, v) in cols {
        all_params.push(match v {
            SqlValue::Null => rusqlite::types::Value::Null,
            SqlValue::Integer(n) => rusqlite::types::Value::Integer(*n),
            SqlValue::Real(f) => rusqlite::types::Value::Real(*f),
            SqlValue::Text(s) => rusqlite::types::Value::Text(s.clone()),
            SqlValue::Blob(b) => rusqlite::types::Value::Blob(b.clone()),
        });
    }
    let result = tx.execute(&sql, rusqlite::params_from_iter(all_params.iter()));
    match result {
        Ok(_) => {
            tx.commit()?;
            Ok(())
        }
        Err(e) => {
            let _ = tx.rollback();
            Err(anyhow::anyhow!("{}", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{clear_table, delete_rows_in_view, insert_row};
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
    fn delete_rows_in_view_respects_order_and_offset() {
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

        let deleted = delete_rows_in_view(&conn, "users", 1, 2, Some(("name", true)), "", &[])
            .expect("delete selected rows");

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

        let deleted = clear_table(&conn, "users").expect("clear table");
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .expect("count rows");

        assert_eq!(deleted, 2);
        assert_eq!(remaining, 0);
    }
}
