pub mod query;
pub mod schema;
pub mod types;
pub mod write;

use anyhow::Context;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, OptionalExtension, Row};

use query::quote_identifier;
use schema::{Column, ForeignKey, IndexMeta, RowIdentity, Schema, TableMeta, ViewMeta};

pub type DbPool = r2d2::Pool<SqliteConnectionManager>;

pub struct RowFetch<'a> {
    pub table: &'a str,
    pub columns: &'a [Column],
    pub offset: i64,
    pub limit: i64,
    pub order_by: Option<(&'a str, bool)>,
    pub where_clause: &'a str,
    pub where_params: &'a [rusqlite::types::Value],
}

fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "regexp",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let pattern: String = ctx.get(0)?;
            let text: Option<String> = ctx.get(1)?;
            let re = regex::Regex::new(&pattern)
                .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
            Ok(text.is_some_and(|text| re.is_match(&text)))
        },
    )?;
    Ok(())
}

pub fn open_pool(path: &str, readonly: bool) -> anyhow::Result<DbPool> {
    let manager = if path == ":memory:" {
        SqliteConnectionManager::memory().with_init(|conn| {
            conn.execute_batch("PRAGMA foreign_keys = ON;")?;
            register_functions(conn)?;
            Ok(())
        })
    } else {
        let flags = if readonly {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
        };
        SqliteConnectionManager::file(path)
            .with_flags(flags)
            .with_init(|conn| {
                conn.execute_batch("PRAGMA foreign_keys = ON;")?;
                register_functions(conn)?;
                Ok(())
            })
    };
    Ok(r2d2::Pool::new(manager)?)
}

pub fn load_schema(conn: &Connection) -> anyhow::Result<Schema> {
    let table_names = load_object_names(conn, "table")?;
    let view_names_sql = load_views_with_sql(conn)?;

    let mut tables = Vec::new();
    for name in &table_names {
        let columns = load_columns(conn, name)?;
        let foreign_keys = load_foreign_keys(conn, name)?;
        let index_names = load_index_names_for_table(conn, name)?;
        let row_identity = load_row_identity(conn, name, &columns)?;
        tables.push(TableMeta {
            name: name.clone(),
            columns,
            foreign_keys,
            indexes: index_names,
            row_identity,
        });
    }

    let views = view_names_sql
        .into_iter()
        .map(|(name, sql)| ViewMeta { name, sql })
        .collect();

    let indexes = load_all_indexes(conn, &table_names)?;

    Ok(Schema {
        tables,
        views,
        indexes,
    })
}

fn load_object_names(conn: &Connection, obj_type: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = ?1 AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([obj_type], |row| row.get::<_, String>(0))?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("loading object names")
}

fn load_views_with_sql(conn: &Connection) -> anyhow::Result<Vec<(String, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT name, sql FROM sqlite_master WHERE type = 'view' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>().context("loading views")
}

pub(crate) fn load_columns(conn: &Connection, table: &str) -> anyhow::Result<Vec<Column>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_xinfo({})", quote_identifier(table)))?;
    let rows = stmt.query_map([], |row| {
        Ok(Column {
            cid: row.get::<_, i64>(0)?,
            name: row.get::<_, String>(1)?,
            col_type: row.get::<_, String>(2)?,
            not_null: row.get::<_, i64>(3)? != 0,
            default_value: row.get::<_, Option<String>>(4)?,
            pk_position: row.get::<_, i64>(5)?,
            is_pk: row.get::<_, i64>(5)? != 0,
            writable: row.get::<_, i64>(6)? == 0,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("loading columns")
}

pub fn load_row_identity(
    conn: &Connection,
    table: &str,
    columns: &[Column],
) -> anyhow::Result<Option<RowIdentity>> {
    let without_rowid: bool = conn
        .query_row(
            "SELECT wr != 0 FROM pragma_table_list WHERE schema = 'main' AND name = ?1 LIMIT 1",
            [table],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false);

    if !without_rowid {
        for alias in ["rowid", "_rowid_", "oid"] {
            if !columns
                .iter()
                .any(|column| column.name.eq_ignore_ascii_case(alias))
            {
                return Ok(Some(RowIdentity::RowidAlias(alias.to_string())));
            }
        }
    }

    let mut primary_key = columns
        .iter()
        .filter(|column| column.pk_position > 0)
        .collect::<Vec<_>>();
    primary_key.sort_by_key(|column| column.pk_position);
    let primary_key = primary_key
        .into_iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    Ok((!primary_key.is_empty()).then_some(RowIdentity::PrimaryKey(primary_key)))
}

fn load_foreign_keys(conn: &Connection, table: &str) -> anyhow::Result<Vec<ForeignKey>> {
    let mut stmt = conn.prepare(&format!(
        "PRAGMA foreign_key_list({})",
        quote_identifier(table)
    ))?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(1)?,
            ForeignKey {
                to_table: row.get::<_, String>(2)?,
                from_col: row.get::<_, String>(3)?,
                to_col: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            },
        ))
    })?;
    let mut fks = rows
        .collect::<Result<Vec<_>, _>>()
        .context("loading foreign keys")?;

    for (sequence, fk) in &mut fks {
        if fk.to_col.is_empty() {
            fk.to_col = resolve_table_pk_col(conn, &fk.to_table, *sequence)?;
        }
    }

    Ok(fks.into_iter().map(|(_, fk)| fk).collect())
}

fn resolve_table_pk_col(conn: &Connection, table: &str, sequence: i64) -> anyhow::Result<String> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_xinfo({})", quote_identifier(table)))?;
    let mut primary_key = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    primary_key.retain(|(_, position)| *position > 0);
    primary_key.sort_by_key(|(_, position)| *position);
    primary_key
        .get(sequence as usize)
        .map(|(name, _)| name.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no primary-key column {} found for table {:?}",
                sequence + 1,
                table
            )
        })
}

fn load_index_names_for_table(conn: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA index_list({})", quote_identifier(table)))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let names: Vec<String> = rows
        .collect::<Result<Vec<_>, _>>()
        .context("loading index names")?
        .into_iter()
        .filter(|n| !n.starts_with("sqlite_"))
        .collect();
    Ok(names)
}

fn load_all_indexes(conn: &Connection, tables: &[String]) -> anyhow::Result<Vec<IndexMeta>> {
    let mut indexes = Vec::new();
    for table in tables {
        let mut stmt = conn.prepare(&format!("PRAGMA index_list({})", quote_identifier(table)))?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?, // name
                row.get::<_, i64>(2)?,    // unique
            ))
        })?;
        for row in rows {
            let (name, unique) = row?;
            if !name.starts_with("sqlite_") {
                indexes.push(IndexMeta {
                    name,
                    table: table.clone(),
                    unique: unique != 0,
                });
            }
        }
    }
    Ok(indexes)
}

pub fn count_rows(
    conn: &Connection,
    table: &str,
    where_clause: &str,
    where_params: &[rusqlite::types::Value],
) -> anyhow::Result<i64> {
    let where_part = build_where_part(where_clause);
    let sql = format!(
        "SELECT COUNT(*) FROM {}{}",
        quote_identifier(table),
        where_part
    );
    let count: i64 = conn.query_row(
        &sql,
        rusqlite::params_from_iter(where_params.iter()),
        |row| row.get(0),
    )?;
    Ok(count)
}

fn build_order_terms(order_by: Option<(&str, bool)>, identity: &RowIdentity) -> String {
    let identity_terms = match identity {
        RowIdentity::RowidAlias(alias) => quote_identifier(alias),
        RowIdentity::PrimaryKey(columns) => columns
            .iter()
            .map(|column| format!("{} ASC", quote_identifier(column)))
            .collect::<Vec<_>>()
            .join(", "),
    };
    match order_by {
        Some((column, asc)) => format!(
            "{} {}, {}",
            quote_identifier(column),
            if asc { "ASC" } else { "DESC" },
            identity_terms
        ),
        None => identity_terms,
    }
}

fn build_where_part(where_clause: &str) -> String {
    if where_clause.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clause)
    }
}

fn unused_column_alias(columns: &[Column], base: &str) -> String {
    let mut candidate = base.to_string();
    let mut suffix = 0_u64;
    while columns
        .iter()
        .any(|column| column.name.eq_ignore_ascii_case(&candidate))
    {
        suffix += 1;
        candidate = format!("{base}_{suffix}");
    }
    candidate
}

fn decode_sql_value(value: rusqlite::types::ValueRef<'_>) -> types::SqlValue {
    use rusqlite::types::ValueRef;
    use types::SqlValue;

    match value {
        ValueRef::Null => SqlValue::Null,
        ValueRef::Integer(n) => SqlValue::Integer(n),
        ValueRef::Real(f) => SqlValue::Real(f),
        ValueRef::Text(bytes) => SqlValue::Text(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => SqlValue::Blob(bytes.to_vec()),
    }
}

fn decode_row_values(row: &Row<'_>, col_count: usize) -> rusqlite::Result<Vec<types::SqlValue>> {
    (0..col_count)
        .map(|index| row.get_ref(index).map(decode_sql_value))
        .collect()
}

pub fn fetch_rows(
    conn: &Connection,
    request: RowFetch<'_>,
) -> anyhow::Result<Vec<Vec<types::SqlValue>>> {
    if request.columns.is_empty() {
        return Ok(Vec::new());
    }

    let identity = load_row_identity(conn, request.table, request.columns)?
        .ok_or_else(|| anyhow::anyhow!("table {:?} has no stable row identity", request.table))?;
    let col_names: Vec<String> = request
        .columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect();
    let order_clause = format!(
        " ORDER BY {}",
        build_order_terms(request.order_by, &identity)
    );
    let where_part = build_where_part(request.where_clause);
    let query = format!(
        "SELECT {} FROM {}{}{} LIMIT {} OFFSET {}",
        col_names.join(", "),
        quote_identifier(request.table),
        where_part,
        order_clause,
        request.limit,
        request.offset
    );

    let mut stmt = conn.prepare(&query)?;
    let col_count = request.columns.len();
    let rows = stmt.query_map(
        rusqlite::params_from_iter(request.where_params.iter()),
        |row| decode_row_values(row, col_count),
    )?;

    rows.collect::<Result<Vec<_>, _>>().context("fetching rows")
}

pub fn visit_rows(
    conn: &Connection,
    request: RowFetch<'_>,
    mut visitor: impl FnMut(&[types::SqlValue]) -> anyhow::Result<()>,
) -> anyhow::Result<u64> {
    if request.columns.is_empty() {
        return Ok(0);
    }
    let identity = load_row_identity(conn, request.table, request.columns)?
        .ok_or_else(|| anyhow::anyhow!("table {:?} has no stable row identity", request.table))?;
    let columns = request
        .columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        "SELECT {} FROM {}{} ORDER BY {} LIMIT ?{} OFFSET ?{}",
        columns,
        quote_identifier(request.table),
        build_where_part(request.where_clause),
        build_order_terms(request.order_by, &identity),
        request.where_params.len() + 1,
        request.where_params.len() + 2,
    );
    let mut params = request.where_params.to_vec();
    params.push(rusqlite::types::Value::Integer(request.limit));
    params.push(rusqlite::types::Value::Integer(request.offset));
    let mut stmt = conn.prepare(&query)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let values = decode_row_values(row, request.columns.len())?;
        visitor(&values)?;
        count += 1;
    }
    Ok(count)
}

pub struct FetchedRows {
    pub rows: Vec<Vec<types::SqlValue>>,
    pub rowids: Vec<Option<i64>>,
}

/// Fetches display values and their physical rowids in the same SQLite snapshot.
/// WITHOUT ROWID tables remain browsable and return `None` rowids, which disables
/// row mutations in the UI.
pub fn fetch_rows_with_rowids(
    conn: &Connection,
    request: RowFetch<'_>,
) -> anyhow::Result<FetchedRows> {
    if request.columns.is_empty() {
        return Ok(FetchedRows {
            rows: Vec::new(),
            rowids: Vec::new(),
        });
    }
    let identity = load_row_identity(conn, request.table, request.columns)?
        .ok_or_else(|| anyhow::anyhow!("table {:?} has no stable row identity", request.table))?;
    let rowid_alias = match &identity {
        RowIdentity::RowidAlias(alias) => Some(alias.as_str()),
        RowIdentity::PrimaryKey(_) => None,
    };
    let mut selections =
        Vec::with_capacity(request.columns.len() + usize::from(rowid_alias.is_some()));
    if let Some(alias) = rowid_alias {
        selections.push(quote_identifier(alias));
    }
    selections.extend(
        request
            .columns
            .iter()
            .map(|column| quote_identifier(&column.name)),
    );
    let query = format!(
        "SELECT {} FROM {}{} ORDER BY {} LIMIT ?{} OFFSET ?{}",
        selections.join(", "),
        quote_identifier(request.table),
        build_where_part(request.where_clause),
        build_order_terms(request.order_by, &identity),
        request.where_params.len() + 1,
        request.where_params.len() + 2,
    );
    let mut params = request.where_params.to_vec();
    params.push(rusqlite::types::Value::Integer(request.limit));
    params.push(rusqlite::types::Value::Integer(request.offset));
    let mut stmt = conn.prepare(&query)?;
    let value_start = usize::from(rowid_alias.is_some());
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        let rowid = if rowid_alias.is_some() {
            Some(row.get(0)?)
        } else {
            None
        };
        let values = (0..request.columns.len())
            .map(|index| row.get_ref(index + value_start).map(decode_sql_value))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rowid, values))
    })?;
    let fetched = rows
        .collect::<Result<Vec<_>, _>>()
        .context("fetching rows")?;
    let (rowids, rows) = fetched.into_iter().unzip();
    Ok(FetchedRows { rows, rowids })
}

pub fn fetch_offset_for_rowid(
    conn: &Connection,
    table: &str,
    rowid: i64,
    order_by: Option<(&str, bool)>,
    where_clause: &str,
    where_params: &[rusqlite::types::Value],
) -> anyhow::Result<Option<i64>> {
    let columns = load_columns(conn, table)?;
    let identity = load_row_identity(conn, table, &columns)?
        .ok_or_else(|| anyhow::anyhow!("table {:?} has no stable row identity", table))?;
    let RowIdentity::RowidAlias(alias) = &identity else {
        return Ok(None);
    };
    let order_terms = build_order_terms(order_by, &identity);
    let where_part = build_where_part(where_clause);
    let rowid_param = where_params.len() + 1;
    let query = format!(
        "SELECT visible_offset FROM (
            SELECT {rowid}, ROW_NUMBER() OVER (ORDER BY {order_terms}) - 1 AS visible_offset
            FROM {table}{where_part}
        ) WHERE {rowid} = ?{rowid_param} LIMIT 1",
        rowid = quote_identifier(alias),
        table = quote_identifier(table)
    );
    let mut params = where_params.to_vec();
    params.push(rusqlite::types::Value::Integer(rowid));
    conn.query_row(&query, rusqlite::params_from_iter(params.iter()), |row| {
        row.get(0)
    })
    .optional()
    .context("fetching offset for rowid")
}

pub fn fetch_rowids_at_offsets(
    conn: &Connection,
    table: &str,
    offsets: &[i64],
    order_by: Option<(&str, bool)>,
    where_clause: &str,
    where_params: &[rusqlite::types::Value],
) -> anyhow::Result<Vec<i64>> {
    if offsets.is_empty() {
        return Ok(Vec::new());
    }
    let columns = load_columns(conn, table)?;
    let identity = load_row_identity(conn, table, &columns)?
        .ok_or_else(|| anyhow::anyhow!("table {:?} has no stable row identity", table))?;
    let RowIdentity::RowidAlias(alias) = &identity else {
        anyhow::bail!(
            "table {:?} does not expose a safe rowid for mutations",
            table
        );
    };
    let offset_list = offsets
        .iter()
        .copied()
        .filter(|offset| *offset >= 0)
        .map(|offset| offset.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if offset_list.is_empty() {
        return Ok(Vec::new());
    }
    let rowid = quote_identifier(alias);
    let query = format!(
        "WITH visible AS (
            SELECT {rowid}, ROW_NUMBER() OVER (ORDER BY {order}) - 1 AS visible_offset
            FROM {table}{where_part}
         )
         SELECT {rowid} FROM visible WHERE visible_offset IN ({offset_list})",
        order = build_order_terms(order_by, &identity),
        table = quote_identifier(table),
        where_part = build_where_part(where_clause),
    );
    let mut stmt = conn.prepare(&query)?;
    let rowids = stmt
        .query_map(rusqlite::params_from_iter(where_params.iter()), |row| {
            row.get(0)
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("resolving selected row identities")?;
    Ok(rowids)
}

pub fn fetch_rows_at_offsets(
    conn: &Connection,
    table: &str,
    columns: &[Column],
    offsets: &[i64],
    order_by: Option<(&str, bool)>,
    where_clause: &str,
    where_params: &[rusqlite::types::Value],
) -> anyhow::Result<Vec<Vec<types::SqlValue>>> {
    if columns.is_empty() || offsets.is_empty() {
        return Ok(Vec::new());
    }
    let identity = load_row_identity(conn, table, columns)?
        .ok_or_else(|| anyhow::anyhow!("table {:?} has no stable row identity", table))?;
    let offset_list = offsets
        .iter()
        .copied()
        .filter(|offset| *offset >= 0)
        .map(|offset| offset.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if offset_list.is_empty() {
        return Ok(Vec::new());
    }
    let inner_columns = columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let outer_columns = columns
        .iter()
        .map(|column| format!("visible.{}", quote_identifier(&column.name)))
        .collect::<Vec<_>>()
        .join(", ");
    let offset_alias = quote_identifier(&unused_column_alias(columns, "__sqview_offset"));
    let query = format!(
        "WITH visible AS (
            SELECT {inner_columns}, ROW_NUMBER() OVER (ORDER BY {order}) - 1 AS {offset_alias}
            FROM {table}{where_part}
         )
         SELECT {outer_columns} FROM visible
         WHERE visible.{offset_alias} IN ({offset_list}) ORDER BY visible.{offset_alias}",
        order = build_order_terms(order_by, &identity),
        table = quote_identifier(table),
        where_part = build_where_part(where_clause),
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(where_params.iter()), |row| {
        decode_row_values(row, columns.len())
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("fetching selected rows")
}

pub fn load_distinct_values(
    conn: &Connection,
    table: &str,
    column: &str,
    limit: usize,
) -> anyhow::Result<Vec<String>> {
    use rusqlite::types::ValueRef;

    let column = quote_identifier(column);
    let sql = format!(
        "SELECT DISTINCT {column} FROM {} WHERE {column} IS NOT NULL ORDER BY 1 LIMIT ?1",
        quote_identifier(table)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit as i64], |row| {
        let value = match row.get_ref(0)? {
            ValueRef::Null => String::new(),
            ValueRef::Integer(n) => n.to_string(),
            ValueRef::Real(f) => f.to_string(),
            ValueRef::Text(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            ValueRef::Blob(bytes) => format!("<blob {} bytes>", bytes.len()),
        };
        Ok(value)
    })?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("loading distinct values")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_column(cid: i64, name: &str) -> Column {
        Column {
            cid,
            name: name.to_string(),
            col_type: "TEXT".to_string(),
            not_null: false,
            default_value: None,
            is_pk: false,
            pk_position: 0,
            writable: true,
        }
    }

    #[test]
    fn fetch_rows_respects_sort_and_filter() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE items (name TEXT, created_at TEXT);
            INSERT INTO items (rowid, name, created_at) VALUES
              (11, 'a', '2024-01-01'),
              (22, 'b', '2024-01-02'),
              (33, 'c', '2024-01-03');
            "#,
        )
        .expect("seed items");

        let columns = vec![text_column(0, "name"), text_column(1, "created_at")];
        let where_params = [rusqlite::types::Value::Text("a".to_string())];
        let rows = fetch_rows(
            &conn,
            RowFetch {
                table: "items",
                columns: &columns,
                offset: 0,
                limit: 2,
                order_by: Some(("created_at", false)),
                where_clause: "\"name\" != ?1",
                where_params: &where_params,
            },
        )
        .expect("row fetch");

        assert_eq!(
            rows,
            vec![
                vec![
                    types::SqlValue::Text("c".to_string()),
                    types::SqlValue::Text("2024-01-03".to_string()),
                ],
                vec![
                    types::SqlValue::Text("b".to_string()),
                    types::SqlValue::Text("2024-01-02".to_string()),
                ],
            ]
        );
    }

    #[test]
    fn fetch_offset_for_rowid_respects_sort_and_filter() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE items (name TEXT, created_at TEXT);
            INSERT INTO items (rowid, name, created_at) VALUES
              (11, 'a', '2024-01-01'),
              (22, 'b', '2024-01-02'),
              (33, 'c', '2024-01-03');
            "#,
        )
        .expect("seed items");

        let offset = fetch_offset_for_rowid(
            &conn,
            "items",
            22,
            Some(("created_at", false)),
            "\"name\" != ?1",
            &[rusqlite::types::Value::Text("a".to_string())],
        )
        .expect("offset lookup");

        assert_eq!(offset, Some(1));
    }

    #[test]
    fn sorted_row_lookup_is_stable_for_duplicate_values() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE items (name TEXT);
            INSERT INTO items (rowid, name) VALUES
              (20, 'same'),
              (10, 'same'),
              (30, 'z');
            "#,
        )
        .expect("seed items");

        let columns = vec![text_column(0, "name")];
        let fetched = fetch_rows_with_rowids(
            &conn,
            RowFetch {
                table: "items",
                columns: &columns,
                offset: 0,
                limit: 1,
                order_by: Some(("name", true)),
                where_clause: "",
                where_params: &[],
            },
        )
        .expect("row fetch");
        let second_offset =
            fetch_offset_for_rowid(&conn, "items", 20, Some(("name", true)), "", &[])
                .expect("offset lookup");

        assert_eq!(fetched.rowids, vec![Some(10)]);
        assert_eq!(second_offset, Some(1));
    }

    #[test]
    fn selected_row_fetch_avoids_internal_alias_collisions() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE items (__sqview_offset INTEGER, value TEXT);
             INSERT INTO items VALUES (99, 'first'), (0, 'second'), (99, 'third');",
        )
        .expect("seed items");
        let columns = load_columns(&conn, "items").expect("load columns");

        let rows = fetch_rows_at_offsets(&conn, "items", &columns, &[0, 2], None, "", &[])
            .expect("fetch selected rows");

        assert_eq!(
            rows,
            vec![
                vec![
                    types::SqlValue::Integer(99),
                    types::SqlValue::Text("first".to_string()),
                ],
                vec![
                    types::SqlValue::Integer(99),
                    types::SqlValue::Text("third".to_string()),
                ],
            ]
        );
    }

    #[test]
    fn without_rowid_tables_are_browsable_in_primary_key_order() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE items (part INTEGER, code TEXT, value TEXT, PRIMARY KEY(part, code)) WITHOUT ROWID;
             INSERT INTO items VALUES (2, 'b', 'second'), (1, 'a', 'first');",
        )
        .expect("seed items");
        let table = load_schema(&conn).expect("schema").tables.remove(0);
        assert_eq!(
            table.row_identity,
            Some(RowIdentity::PrimaryKey(vec![
                "part".to_string(),
                "code".to_string()
            ]))
        );
        let rows = fetch_rows(
            &conn,
            RowFetch {
                table: &table.name,
                columns: &table.columns,
                offset: 0,
                limit: 10,
                order_by: None,
                where_clause: "",
                where_params: &[],
            },
        )
        .expect("fetch rows");
        assert_eq!(rows[0][2], types::SqlValue::Text("first".to_string()));
    }

    #[test]
    fn schema_preserves_composite_foreign_keys_and_generated_columns() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE parent (a INTEGER, b INTEGER, PRIMARY KEY(a, b));
             CREATE TABLE child (
               a INTEGER,
               b INTEGER,
               sum INTEGER GENERATED ALWAYS AS (a + b) STORED,
               FOREIGN KEY(a, b) REFERENCES parent
             );",
        )
        .expect("create schema");
        let schema = load_schema(&conn).expect("schema");
        let child = schema
            .tables
            .iter()
            .find(|table| table.name == "child")
            .expect("child table");
        assert_eq!(
            child
                .foreign_keys
                .iter()
                .map(|foreign_key| (foreign_key.from_col.as_str(), foreign_key.to_col.as_str()))
                .collect::<Vec<_>>(),
            vec![("a", "a"), ("b", "b")]
        );
        assert!(
            !child
                .columns
                .iter()
                .find(|column| column.name == "sum")
                .expect("generated column")
                .writable
        );
    }

    #[test]
    fn regexp_treats_null_as_a_non_match() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        register_functions(&conn).expect("register regexp");
        let matched: bool = conn
            .query_row("SELECT regexp('x', NULL)", [], |row| row.get(0))
            .expect("regexp result");
        assert!(!matched);
    }
}
