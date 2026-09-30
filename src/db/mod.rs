pub mod query;
pub mod schema;
pub mod types;
pub mod write;

use anyhow::Context;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, OptionalExtension, Row};

use query::{quote_identifier, ViewQuery};
use rusqlite::types::Value;
use schema::{Column, ForeignKey, RowIdentity, Schema, TableMeta};
use types::SqlValue;

pub type DbPool = r2d2::Pool<SqliteConnectionManager>;

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
    fn init(conn: &mut Connection) -> rusqlite::Result<()> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        register_functions(conn)
    }

    let manager = if path == ":memory:" {
        SqliteConnectionManager::memory()
    } else {
        let access = if readonly {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
        };
        SqliteConnectionManager::file(path)
            .with_flags(access | rusqlite::OpenFlags::SQLITE_OPEN_URI)
    };
    Ok(r2d2::Pool::new(manager.with_init(init))?)
}

pub fn load_schema(conn: &Connection) -> anyhow::Result<Schema> {
    let table_names = load_object_names(conn, "table")?;
    let mut tables = Vec::with_capacity(table_names.len());
    for name in &table_names {
        let columns = load_columns(conn, name)?;
        tables.push(TableMeta {
            name: name.clone(),
            is_view: false,
            foreign_keys: load_foreign_keys(conn, name)?,
            row_identity: load_row_identity(conn, name, &columns)?,
            columns,
        });
    }
    let mut views = Vec::new();
    for name in load_object_names(conn, "view")? {
        // A view whose definition no longer compiles still gets listed.
        let columns = load_columns(conn, &name).unwrap_or_default();
        views.push(TableMeta {
            name,
            is_view: true,
            columns,
            foreign_keys: Vec::new(),
            row_identity: None,
        });
    }
    Ok(Schema {
        tables,
        views,
        indexes: load_index_names(conn, &table_names)?,
    })
}

/// The `CREATE` statement of a table, view or index.
pub fn load_ddl(conn: &Connection, name: &str) -> anyhow::Result<Option<String>> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE name = ?1 AND sql IS NOT NULL LIMIT 1",
        [name],
        |row| row.get(0),
    )
    .optional()
    .context("loading schema definition")
}

/// An index of a table: its name, whether it is unique and its columns.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexInfo {
    pub name: String,
    pub unique: bool,
    pub columns: Vec<String>,
}

pub fn load_indexes(conn: &Connection, table: &str) -> anyhow::Result<Vec<IndexInfo>> {
    let mut list = conn.prepare(&format!("PRAGMA index_list({})", quote_identifier(table)))?;
    let entries = list
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(2)? != 0))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut indexes = Vec::with_capacity(entries.len());
    for (name, unique) in entries {
        let mut info = conn.prepare(&format!("PRAGMA index_info({})", quote_identifier(&name)))?;
        let columns = info
            .query_map([], |row| row.get::<_, Option<String>>(2))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|column| column.unwrap_or_else(|| "<expression>".to_string()))
            .collect();
        indexes.push(IndexInfo {
            name,
            unique,
            columns,
        });
    }
    Ok(indexes)
}

/// The table an index belongs to.
pub fn index_table(conn: &Connection, index: &str) -> anyhow::Result<Option<String>> {
    conn.query_row(
        "SELECT tbl_name FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [index],
        |row| row.get(0),
    )
    .optional()
    .context("resolving index table")
}

fn load_object_names(conn: &Connection, obj_type: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = ?1 AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([obj_type], |row| row.get::<_, String>(0))?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("loading object names")
}

pub(crate) fn load_columns(conn: &Connection, table: &str) -> anyhow::Result<Vec<Column>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_xinfo({})", quote_identifier(table)))?;
    let rows = stmt.query_map([], |row| {
        Ok(Column {
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
    let (kind, without_rowid): (String, bool) = conn
        .query_row(
            "SELECT type, wr != 0 FROM pragma_table_list WHERE schema = 'main' AND name = ?1 LIMIT 1",
            [table],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .unwrap_or_else(|| ("table".to_string(), false));
    if kind != "table" {
        return Ok(None);
    }

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

    let primary_key = primary_key_columns(columns);
    Ok((!primary_key.is_empty()).then_some(RowIdentity::PrimaryKey(primary_key)))
}

/// Primary-key column names in key order.
fn primary_key_columns(columns: &[Column]) -> Vec<String> {
    let mut primary_key = columns
        .iter()
        .filter(|column| column.pk_position > 0)
        .collect::<Vec<_>>();
    primary_key.sort_by_key(|column| column.pk_position);
    primary_key
        .into_iter()
        .map(|column| column.name.clone())
        .collect()
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
    primary_key_columns(&load_columns(conn, table)?)
        .into_iter()
        .nth(sequence as usize)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no primary-key column {} found for table {:?}",
                sequence + 1,
                table
            )
        })
}

fn load_index_names(conn: &Connection, tables: &[String]) -> anyhow::Result<Vec<String>> {
    let mut indexes = Vec::new();
    for table in tables {
        let mut stmt = conn.prepare(&format!("PRAGMA index_list({})", quote_identifier(table)))?;
        let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for name in names {
            let name = name.context("loading index names")?;
            if !name.starts_with("sqlite_") {
                indexes.push(name);
            }
        }
    }
    Ok(indexes)
}

fn stable_identity(
    conn: &Connection,
    table: &str,
    columns: &[Column],
) -> anyhow::Result<RowIdentity> {
    load_row_identity(conn, table, columns)?
        .ok_or_else(|| anyhow::anyhow!("table {:?} has no stable row identity", table))
}

/// The ` ORDER BY` clause for reading `view`; empty for a view without sort keys
/// or row identity, whose row order SQLite then chooses.
fn order_clause(view: &ViewQuery, identity: Option<&RowIdentity>) -> String {
    let terms = view.order_terms(identity);
    if terms.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {terms}")
    }
}

fn mutable_rowid_alias(identity: &RowIdentity, table: &str) -> anyhow::Result<String> {
    match identity {
        RowIdentity::RowidAlias(alias) => Ok(alias.clone()),
        RowIdentity::PrimaryKey(_) => anyhow::bail!(
            "table {:?} does not expose a safe rowid for mutations",
            table
        ),
    }
}

/// The rowid alias through which rows of `table` may be mutated, rejecting tables
/// whose rows are only identified by a primary key.
pub(crate) fn mutation_rowid_alias(conn: &Connection, table: &str) -> anyhow::Result<String> {
    let columns = load_columns(conn, table)?;
    mutable_rowid_alias(&stable_identity(conn, table, &columns)?, table)
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

pub fn count_rows(conn: &Connection, view: &ViewQuery) -> anyhow::Result<i64> {
    let sql = format!(
        "SELECT COUNT(*) FROM {}{}",
        view.quoted_table(),
        view.where_part()
    );
    let count = conn.query_row(
        &sql,
        rusqlite::params_from_iter(view.where_params.iter()),
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Offset of the first row whose sort value starts at `letter` in a text-sorted
/// view; `'#'` addresses the leading NULL (ascending) or trailing non-digit
/// (descending) block.
pub fn count_rows_before_letter(
    conn: &Connection,
    view: &ViewQuery,
    letter: char,
) -> anyhow::Result<i64> {
    let order = view
        .order_by
        .first()
        .ok_or_else(|| anyhow::anyhow!("letter navigation requires a sorted column"))?;
    let column = quote_identifier(&order.column);
    let upper = letter.to_uppercase().next().unwrap_or(letter);
    let first = view.where_params.len() + 1;
    let (predicate, extra) = match (order.ascending, letter == '#') {
        (true, true) => (format!("{column} IS NULL"), Vec::new()),
        (true, false) => (
            format!("({column} IS NULL OR {column} < ?{first})"),
            vec![Value::Text(upper.to_string())],
        ),
        (false, true) => (
            format!("({column} IS NOT NULL AND {column} NOT GLOB '[0-9]*')"),
            Vec::new(),
        ),
        (false, false) => (
            format!("({column} > ?{first} AND {column} NOT LIKE ?{})", first + 1),
            vec![
                Value::Text(upper.to_string()),
                Value::Text(format!("{upper}%")),
            ],
        ),
    };
    let where_clause = if view.where_clause.is_empty() {
        predicate
    } else {
        format!("({}) AND {predicate}", view.where_clause)
    };
    let (params, _) = view.params_with(extra);
    let sql = format!(
        "SELECT COUNT(*) FROM {} WHERE {where_clause}",
        view.quoted_table()
    );
    let count = conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |row| {
        row.get(0)
    })?;
    Ok(count)
}

pub(crate) fn decode_row_values(
    row: &Row<'_>,
    col_count: usize,
) -> rusqlite::Result<Vec<SqlValue>> {
    decode_values_from(row, 0, col_count)
}

fn decode_values_from(
    row: &Row<'_>,
    start: usize,
    count: usize,
) -> rusqlite::Result<Vec<SqlValue>> {
    (start..start + count)
        .map(|index| row.get_ref(index).map(SqlValue::from))
        .collect()
}

/// Streams one page of `view`, reading each row's physical rowid in the same
/// statement as its values. WITHOUT ROWID tables yield `None` rowids, which
/// disables row mutations in the UI.
fn select_rows(
    conn: &Connection,
    view: &ViewQuery,
    columns: &[Column],
    offset: i64,
    limit: i64,
    mut visit: impl FnMut(Option<i64>, Vec<SqlValue>) -> anyhow::Result<()>,
) -> anyhow::Result<u64> {
    if columns.is_empty() {
        return Ok(0);
    }
    let identity = load_row_identity(conn, &view.table, columns)?;
    let rowid_alias = match &identity {
        Some(RowIdentity::RowidAlias(alias)) => Some(alias.as_str()),
        _ => None,
    };
    let selections = rowid_alias
        .into_iter()
        .chain(columns.iter().map(|column| column.name.as_str()))
        .map(quote_identifier)
        .collect::<Vec<_>>()
        .join(", ");
    let (params, first) = view.params_with([Value::Integer(limit), Value::Integer(offset)]);
    let query = format!(
        "SELECT {selections} FROM {}{}{} LIMIT ?{first} OFFSET ?{}",
        view.quoted_table(),
        view.where_part(),
        order_clause(view, identity.as_ref()),
        first + 1,
    );
    let value_start = usize::from(rowid_alias.is_some());
    let mut stmt = conn.prepare(&query)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let rowid = rowid_alias.map(|_| row.get(0)).transpose()?;
        visit(rowid, decode_values_from(row, value_start, columns.len())?)?;
        count += 1;
    }
    Ok(count)
}

#[derive(Debug, Default)]
pub struct FetchedRows {
    pub rows: Vec<Vec<SqlValue>>,
    pub rowids: Vec<Option<i64>>,
}

pub fn fetch_rows(
    conn: &Connection,
    view: &ViewQuery,
    columns: &[Column],
    offset: i64,
    limit: i64,
) -> anyhow::Result<FetchedRows> {
    let mut fetched = FetchedRows::default();
    select_rows(conn, view, columns, offset, limit, |rowid, values| {
        fetched.rowids.push(rowid);
        fetched.rows.push(values);
        Ok(())
    })
    .context("fetching rows")?;
    Ok(fetched)
}

/// Visits every row of `view` without materializing the result set.
pub fn visit_rows(
    conn: &Connection,
    view: &ViewQuery,
    columns: &[Column],
    mut visitor: impl FnMut(Vec<SqlValue>) -> anyhow::Result<()>,
) -> anyhow::Result<u64> {
    select_rows(conn, view, columns, 0, i64::MAX, |_, values| {
        visitor(values)
    })
}

pub fn fetch_offset_for_rowid(
    conn: &Connection,
    view: &ViewQuery,
    rowid: i64,
) -> anyhow::Result<Option<i64>> {
    let columns = load_columns(conn, &view.table)?;
    let identity = stable_identity(conn, &view.table, &columns)?;
    let RowIdentity::RowidAlias(alias) = &identity else {
        return Ok(None);
    };
    let (params, rowid_param) = view.params_with([Value::Integer(rowid)]);
    let query = format!(
        "SELECT visible_offset FROM (
            SELECT {rowid}, ROW_NUMBER() OVER (ORDER BY {order_terms}) - 1 AS visible_offset
            FROM {table}{where_part}
        ) WHERE {rowid} = ?{rowid_param} LIMIT 1",
        rowid = quote_identifier(alias),
        order_terms = view.order_terms(Some(&identity)),
        table = view.quoted_table(),
        where_part = view.where_part(),
    );
    conn.query_row(&query, rusqlite::params_from_iter(params.iter()), |row| {
        row.get(0)
    })
    .optional()
    .context("fetching offset for rowid")
}

fn offset_list(offsets: &[i64]) -> Option<String> {
    let list = offsets
        .iter()
        .filter(|offset| **offset >= 0)
        .map(|offset| offset.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    (!list.is_empty()).then_some(list)
}

pub fn fetch_rowids_at_offsets(
    conn: &Connection,
    view: &ViewQuery,
    offsets: &[i64],
) -> anyhow::Result<Vec<i64>> {
    let Some(offset_list) = offset_list(offsets) else {
        return Ok(Vec::new());
    };
    let columns = load_columns(conn, &view.table)?;
    let identity = stable_identity(conn, &view.table, &columns)?;
    let rowid = quote_identifier(&mutable_rowid_alias(&identity, &view.table)?);
    let query = format!(
        "WITH visible AS (
            SELECT {rowid}, ROW_NUMBER() OVER (ORDER BY {order}) - 1 AS visible_offset
            FROM {table}{where_part}
         )
         SELECT {rowid} FROM visible WHERE visible_offset IN ({offset_list})",
        order = view.order_terms(Some(&identity)),
        table = view.quoted_table(),
        where_part = view.where_part(),
    );
    let mut stmt = conn.prepare(&query)?;
    let rowids = stmt
        .query_map(
            rusqlite::params_from_iter(view.where_params.iter()),
            |row| row.get(0),
        )?
        .collect::<Result<Vec<_>, _>>()
        .context("resolving selected row identities")?;
    Ok(rowids)
}

pub fn fetch_rows_at_offsets(
    conn: &Connection,
    view: &ViewQuery,
    columns: &[Column],
    offsets: &[i64],
) -> anyhow::Result<Vec<Vec<SqlValue>>> {
    let Some(offset_list) = offset_list(offsets).filter(|_| !columns.is_empty()) else {
        return Ok(Vec::new());
    };
    let identity = load_row_identity(conn, &view.table, columns)?;
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
            SELECT {inner_columns}, ROW_NUMBER() OVER ({order}) - 1 AS {offset_alias}
            FROM {table}{where_part}
         )
         SELECT {outer_columns} FROM visible
         WHERE visible.{offset_alias} IN ({offset_list}) ORDER BY visible.{offset_alias}",
        order = order_clause(view, identity.as_ref()).trim_start(),
        table = view.quoted_table(),
        where_part = view.where_part(),
    );
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(view.where_params.iter()),
        |row| decode_row_values(row, columns.len()),
    )?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("fetching selected rows")
}

/// A row of a view that matched a search, with its position in the view.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub offset: i64,
    pub rowid: Option<i64>,
    pub values: Vec<SqlValue>,
}

/// Rows of `view` in which any of `columns` contains `needle` (case-insensitive
/// for ASCII), in view order, at most `limit`. An empty needle returns the first
/// rows. Each hit carries its offset in the view so callers can jump to it.
pub fn search_view(
    conn: &Connection,
    view: &ViewQuery,
    columns: &[Column],
    needle: &str,
    limit: i64,
) -> anyhow::Result<Vec<SearchHit>> {
    if columns.is_empty() {
        return Ok(Vec::new());
    }
    let identity = load_row_identity(conn, &view.table, columns)?;
    let offset_alias = quote_identifier(&unused_column_alias(columns, "__sqview_offset"));
    let rowid_alias = quote_identifier(&unused_column_alias(columns, "__sqview_rowid"));
    let rowid_source = match &identity {
        Some(RowIdentity::RowidAlias(alias)) => quote_identifier(alias),
        _ => "NULL".to_string(),
    };
    let names = columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>();
    let mut extra = Vec::new();
    let matcher = if needle.is_empty() {
        String::new()
    } else {
        let placeholder = view.where_params.len() + 1;
        extra.push(Value::Text(format!("%{}%", escape_like(needle))));
        let tests = names
            .iter()
            .map(|name| format!("CAST(visible.{name} AS TEXT) LIKE ?{placeholder} ESCAPE '\\'"))
            .collect::<Vec<_>>();
        format!(" WHERE {}", tests.join(" OR "))
    };
    extra.push(Value::Integer(limit));
    let (params, first) = view.params_with(extra);
    let limit_param = first + usize::from(!needle.is_empty());
    let query = format!(
        "WITH visible AS (
            SELECT {rowid_source} AS {rowid_alias}, {cols},
                   ROW_NUMBER() OVER ({order}) - 1 AS {offset_alias}
            FROM {table}{where_part}
         )
         SELECT visible.{offset_alias}, visible.{rowid_alias}, {outer} FROM visible{matcher}
         ORDER BY visible.{offset_alias} LIMIT ?{limit_param}",
        cols = names.join(", "),
        outer = names
            .iter()
            .map(|name| format!("visible.{name}"))
            .collect::<Vec<_>>()
            .join(", "),
        order = order_clause(view, identity.as_ref()).trim_start(),
        table = view.quoted_table(),
        where_part = view.where_part(),
    );
    let mut stmt = conn.prepare(&query)?;
    let hits = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(SearchHit {
            offset: row.get(0)?,
            rowid: row.get(1)?,
            values: decode_values_from(row, 2, columns.len())?,
        })
    })?;
    hits.collect::<Result<Vec<_>, _>>()
        .context("searching rows")
}

fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// The result of a statement typed into the SQL console.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlOutcome {
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<SqlValue>>,
        /// Whether rows beyond the limit were left out.
        truncated: bool,
    },
    Changed(usize),
}

/// Runs one statement. Queries return up to `limit` rows; statements that
/// modify the database are refused unless `allow_writes`.
pub fn run_sql(
    conn: &Connection,
    sql: &str,
    limit: usize,
    allow_writes: bool,
) -> anyhow::Result<SqlOutcome> {
    let sql = sql.trim().trim_end_matches(';');
    if sql.is_empty() {
        anyhow::bail!("Type a statement to run");
    }
    let mut stmt = conn.prepare(sql)?;
    if !allow_writes && !stmt.readonly() {
        anyhow::bail!("The database is read-only; only queries can run");
    }
    if stmt.column_count() == 0 {
        return Ok(SqlOutcome::Changed(stmt.execute([])?));
    }
    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let count = columns.len();
    let mut rows = Vec::new();
    let mut cursor = stmt.query([])?;
    let mut truncated = false;
    while let Some(row) = cursor.next()? {
        if rows.len() == limit {
            truncated = true;
            break;
        }
        rows.push(decode_row_values(row, count)?);
    }
    Ok(SqlOutcome::Rows {
        columns,
        rows,
        truncated,
    })
}

/// Rows scanned per table to judge which columns are enum-like.
pub const ENUM_SAMPLE_ROWS: i64 = 5000;
const ENUM_MAX_VALUES: i64 = 16;
const ENUM_MAX_LENGTH: i64 = 24;
const ENUM_MIN_VALUES: i64 = 8;

/// The value set of every enum-like column among `columns`: few, short
/// distinct values that actually repeat within the first rows, and no unique
/// index. Other columns get an empty set. One bounded scan decides for all
/// columns; qualifying columns then read their distinct values.
pub fn enum_value_sets(
    conn: &Connection,
    table: &str,
    columns: &[Column],
) -> anyhow::Result<Vec<Vec<String>>> {
    let mut sets = vec![Vec::new(); columns.len()];
    if columns.is_empty() {
        return Ok(sets);
    }
    let unique: Vec<String> = load_indexes(conn, table)?
        .into_iter()
        .filter(|index| index.unique && index.columns.len() == 1)
        .flat_map(|index| index.columns)
        .collect();
    let names: Vec<String> = columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect();
    let stats = names
        .iter()
        .map(|name| format!("COUNT({name}), COUNT(DISTINCT {name}), MAX(LENGTH({name}))"))
        .collect::<Vec<_>>()
        .join(", ");
    let sample = format!(
        "(SELECT {} FROM {} LIMIT {ENUM_SAMPLE_ROWS})",
        names.join(", "),
        quote_identifier(table)
    );
    let counts: Vec<(i64, i64, i64)> =
        conn.query_row(&format!("SELECT {stats} FROM {sample}"), [], |row| {
            (0..columns.len())
                .map(|i| {
                    Ok((
                        row.get(i * 3)?,
                        row.get(i * 3 + 1)?,
                        row.get::<_, Option<i64>>(i * 3 + 2)?.unwrap_or(0),
                    ))
                })
                .collect()
        })?;
    for (index, (non_null, distinct, max_length)) in counts.into_iter().enumerate() {
        let enum_like = distinct <= ENUM_MAX_VALUES
            && non_null >= ENUM_MIN_VALUES
            && distinct * 10 <= non_null * 3
            && max_length <= ENUM_MAX_LENGTH
            && !unique.iter().any(|name| name == &columns[index].name);
        if !enum_like {
            continue;
        }
        let name = &names[index];
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT {name} FROM {sample} WHERE {name} IS NOT NULL ORDER BY 1 LIMIT {ENUM_MAX_VALUES}"
        ))?;
        sets[index] = stmt
            .query_map([], |row| {
                Ok(SqlValue::from(row.get_ref(0)?).to_text().into_owned())
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(sets)
}

/// Resolves the rowid of the first row whose `column` equals `value`, e.g. the
/// target of a foreign-key reference. `None` when no such row exists or the
/// table has no navigable rowid.
pub fn find_rowid_by_value(
    conn: &Connection,
    table: &str,
    column: &str,
    value: &SqlValue,
) -> anyhow::Result<Option<i64>> {
    let columns = load_columns(conn, table)?;
    let Some(RowIdentity::RowidAlias(alias)) = load_row_identity(conn, table, &columns)? else {
        return Ok(None);
    };
    let sql = format!(
        "SELECT {} FROM {} WHERE {} = ?1 LIMIT 1",
        quote_identifier(&alias),
        quote_identifier(table),
        quote_identifier(column)
    );
    conn.query_row(&sql, [value], |row| row.get(0))
        .optional()
        .context("resolving referenced row")
}

pub fn load_distinct_values(
    conn: &Connection,
    table: &str,
    column: &str,
    limit: usize,
) -> anyhow::Result<Vec<String>> {
    let column = quote_identifier(column);
    let sql = format!(
        "SELECT DISTINCT {column} FROM {} WHERE {column} IS NOT NULL ORDER BY 1 LIMIT ?1",
        quote_identifier(table)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit as i64], |row| {
        Ok(SqlValue::from(row.get_ref(0)?).to_text().into_owned())
    })?;

    rows.collect::<Result<Vec<_>, _>>()
        .context("loading distinct values")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_column(name: &str) -> Column {
        Column {
            name: name.to_string(),
            col_type: "TEXT".to_string(),
            not_null: false,
            default_value: None,
            is_pk: false,
            pk_position: 0,
            writable: true,
        }
    }

    /// `items` sorted by `created_at` descending, excluding name 'a'.
    fn filtered_view() -> ViewQuery {
        ViewQuery {
            order_by: vec![query::OrderBy {
                column: "created_at".to_string(),
                ascending: false,
            }],
            where_clause: "\"name\" != ?1".to_string(),
            where_params: vec![Value::Text("a".to_string())],
            ..ViewQuery::table("items")
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

        let columns = vec![text_column("name"), text_column("created_at")];
        let rows = fetch_rows(&conn, &filtered_view(), &columns, 0, 2)
            .expect("row fetch")
            .rows;

        assert_eq!(
            rows,
            vec![
                vec![
                    SqlValue::Text("c".to_string()),
                    SqlValue::Text("2024-01-03".to_string()),
                ],
                vec![
                    SqlValue::Text("b".to_string()),
                    SqlValue::Text("2024-01-02".to_string()),
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

        let offset = fetch_offset_for_rowid(&conn, &filtered_view(), 22).expect("offset lookup");

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

        let columns = vec![text_column("name")];
        let view = ViewQuery {
            order_by: vec![query::OrderBy {
                column: "name".to_string(),
                ascending: true,
            }],
            ..ViewQuery::table("items")
        };
        let fetched = fetch_rows(&conn, &view, &columns, 0, 1).expect("row fetch");
        let second_offset = fetch_offset_for_rowid(&conn, &view, 20).expect("offset lookup");

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

        let rows = fetch_rows_at_offsets(&conn, &ViewQuery::table("items"), &columns, &[0, 2])
            .expect("fetch selected rows");

        assert_eq!(
            rows,
            vec![
                vec![SqlValue::Integer(99), SqlValue::Text("first".to_string()),],
                vec![SqlValue::Integer(99), SqlValue::Text("third".to_string()),],
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
        let rows = fetch_rows(&conn, &ViewQuery::table(&table.name), &table.columns, 0, 10)
            .expect("fetch rows")
            .rows;
        assert_eq!(rows[0][2], SqlValue::Text("first".to_string()));
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
    fn letter_navigation_counts_only_rows_in_the_view() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE items (name TEXT, category TEXT);
             INSERT INTO items VALUES
                ('Alpha', 'kept'),
                ('Bravo', 'hidden'),
                ('Charlie', 'kept'),
                ('Delta', 'kept');",
        )
        .expect("seed rows");
        let view = ViewQuery {
            order_by: vec![query::OrderBy {
                column: "name".to_string(),
                ascending: true,
            }],
            where_clause: "\"category\" = ?1".to_string(),
            where_params: vec![Value::Text("kept".to_string())],
            ..ViewQuery::table("items")
        };

        let offset = count_rows_before_letter(&conn, &view, 'c').expect("count offset");

        assert_eq!(offset, 1);
    }

    #[test]
    fn search_returns_view_offsets_of_matching_rows() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE items (name TEXT, qty INTEGER);
             INSERT INTO items VALUES ('Apple', 5), ('banana', 50), ('Cherry', 7), ('50% off', 1);",
        )
        .expect("seed rows");
        let columns = load_columns(&conn, "items").expect("columns");
        let view = ViewQuery::table("items");

        let hits = search_view(&conn, &view, &columns, "AN", 10).expect("search");
        assert_eq!(
            hits.iter().map(|hit| hit.offset).collect::<Vec<_>>(),
            vec![1]
        );
        let numbers = search_view(&conn, &view, &columns, "50", 10).expect("search");
        assert_eq!(numbers.len(), 2, "numbers match as text");
        let literal = search_view(&conn, &view, &columns, "50%", 10).expect("search");
        assert_eq!(literal.len(), 1, "wildcards in the needle are literal");
        assert_eq!(literal[0].rowid, Some(4));
        assert_eq!(
            search_view(&conn, &view, &columns, "", 2)
                .expect("search")
                .len(),
            2
        );
    }

    #[test]
    fn enum_detection_needs_repetition_and_skips_unique_columns() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE t (status TEXT, fax TEXT, code TEXT UNIQUE);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 40)
             INSERT INTO t SELECT CASE i % 3 WHEN 0 THEN 'new' WHEN 1 THEN 'open' ELSE 'done' END,
                                  CASE WHEN i < 12 THEN 'fax-' || i END,
                                  CASE i % 2 WHEN 0 THEN 'a' || i END FROM n;",
        )
        .expect("seed rows");
        let columns = load_columns(&conn, "t").expect("columns");
        let sets = enum_value_sets(&conn, "t", &columns).expect("enum sets");
        assert_eq!(sets[0], vec!["done", "new", "open"]);
        assert!(
            sets[1].is_empty(),
            "values that never repeat are not an enum"
        );
        assert!(sets[2].is_empty(), "unique columns are not an enum");
    }

    #[test]
    fn views_are_listed_with_columns_and_read_without_identity() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE t (a INTEGER, b TEXT);
             INSERT INTO t VALUES (1, 'x'), (2, 'y');
             CREATE VIEW v AS SELECT b, a * 10 AS scaled FROM t;",
        )
        .expect("seed rows");
        let schema = load_schema(&conn).expect("schema");
        let view = schema.relation("v").expect("view");
        assert!(view.is_view && view.row_identity.is_none());
        let fetched =
            fetch_rows(&conn, &ViewQuery::table("v"), &view.columns, 0, 10).expect("rows");
        assert_eq!(fetched.rows.len(), 2);
        assert_eq!(fetched.rowids, vec![None, None]);
        assert_eq!(count_rows(&conn, &ViewQuery::table("v")).expect("count"), 2);
    }

    #[test]
    fn sql_console_runs_queries_and_guards_writes() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("CREATE TABLE t (a INTEGER); INSERT INTO t VALUES (1), (2), (3);")
            .expect("seed");
        match run_sql(&conn, "SELECT a FROM t ORDER BY a;", 2, false).expect("query") {
            SqlOutcome::Rows {
                columns,
                rows,
                truncated,
            } => {
                assert_eq!(columns, vec!["a"]);
                assert_eq!(rows.len(), 2);
                assert!(truncated);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(run_sql(&conn, "DELETE FROM t", 10, false).is_err());
        assert_eq!(
            run_sql(&conn, "DELETE FROM t WHERE a > 1", 10, true).expect("delete"),
            SqlOutcome::Changed(2)
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
