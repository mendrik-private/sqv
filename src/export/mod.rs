use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rusqlite::Connection;

use crate::db::{
    query::{quote_identifier, ViewQuery},
    schema::Column,
    types::SqlValue,
    visit_rows,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
    Sql,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            ExportFormat::Sql => "sql",
        }
    }
}

/// Which rows an export writes.
pub enum ExportRows<'a> {
    /// Every row of the view, streamed.
    View(&'a ViewQuery),
    /// Rows already read, e.g. a selection.
    Given(&'a [Vec<SqlValue>]),
}

/// Writes `rows` of `table` into `path`, replacing it atomically, and returns
/// the number of exported rows.
pub fn export(
    conn: &Connection,
    format: ExportFormat,
    table: &str,
    columns: &[Column],
    rows: ExportRows<'_>,
    path: &Path,
) -> anyhow::Result<u64> {
    atomic_export(path, |file| {
        write_rows(format, table, columns, file, |emit| match rows {
            ExportRows::View(view) => visit_rows(conn, view, columns, |row| emit(&row)),
            ExportRows::Given(rows) => emit_all(rows, emit),
        })
    })
}

/// `rows` as text in `format`, for the clipboard.
pub fn rows_to_text(
    format: ExportFormat,
    table: &str,
    columns: &[Column],
    rows: &[Vec<SqlValue>],
) -> anyhow::Result<String> {
    let mut out = Vec::new();
    write_rows(format, table, columns, &mut out, |emit| {
        emit_all(rows, emit)
    })?;
    Ok(String::from_utf8(out)?)
}

type Emit<'a> = dyn FnMut(&[SqlValue]) -> anyhow::Result<()> + 'a;

fn emit_all(rows: &[Vec<SqlValue>], emit: &mut Emit<'_>) -> anyhow::Result<u64> {
    for row in rows {
        emit(row)?;
    }
    Ok(rows.len() as u64)
}

/// Writes the rows that `drive` feeds to its emitter in `format`.
fn write_rows<W: Write>(
    format: ExportFormat,
    table: &str,
    columns: &[Column],
    out: W,
    drive: impl FnOnce(&mut Emit<'_>) -> anyhow::Result<u64>,
) -> anyhow::Result<u64> {
    match format {
        ExportFormat::Csv => {
            let mut writer = csv::Writer::from_writer(out);
            writer.write_record(columns.iter().map(|column| column.name.as_str()))?;
            let count = drive(&mut |row| {
                writer.write_record(row.iter().map(val_to_str))?;
                Ok(())
            })?;
            writer.flush()?;
            Ok(count)
        }
        ExportFormat::Json => {
            let mut out = BufWriter::new(out);
            out.write_all(b"[")?;
            let mut first = true;
            let count = drive(&mut |row| {
                if !first {
                    out.write_all(b",")?;
                }
                first = false;
                serde_json::to_writer(&mut out, &row_to_json(columns, row))?;
                Ok(())
            })?;
            out.write_all(b"]\n")?;
            out.flush()?;
            Ok(count)
        }
        ExportFormat::Sql => {
            let col_names = columns
                .iter()
                .map(|column| quote_identifier(&column.name))
                .collect::<Vec<_>>()
                .join(", ");
            let quoted_table = quote_identifier(table);
            let mut out = BufWriter::new(out);
            let count = drive(&mut |row| {
                let values = row.iter().map(val_to_sql_literal).collect::<Vec<_>>();
                writeln!(
                    out,
                    "INSERT INTO {quoted_table} ({col_names}) VALUES ({});",
                    values.join(", ")
                )?;
                Ok(())
            })?;
            out.flush()?;
            Ok(count)
        }
    }
}

/// One row as a JSON object keyed by column name. BLOBs have no JSON
/// representation and become `null`.
pub fn row_to_json(columns: &[Column], row: &[SqlValue]) -> serde_json::Value {
    columns
        .iter()
        .zip(row)
        .map(|(column, value)| (column.name.clone(), val_to_json(value)))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

static NEXT_EXPORT_ID: AtomicU64 = AtomicU64::new(0);

fn atomic_export(
    path: &Path,
    write: impl FnOnce(File) -> anyhow::Result<u64>,
) -> anyhow::Result<u64> {
    let temporary = temporary_export_path(path);
    let result = File::create(&temporary)
        .map_err(anyhow::Error::from)
        .and_then(write);
    match result {
        Ok(count) => {
            std::fs::rename(&temporary, path)?;
            Ok(count)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn temporary_export_path(path: &Path) -> PathBuf {
    let id = NEXT_EXPORT_ID.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    path.with_file_name(format!(".{file_name}.{}-{id}.tmp", std::process::id()))
}

/// CSV has no NULL; an empty field is the conventional spelling.
fn val_to_str(v: &SqlValue) -> String {
    match v {
        SqlValue::Null => String::new(),
        value => value.to_text().into_owned(),
    }
}

fn val_to_json(v: &SqlValue) -> serde_json::Value {
    match v {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(n) => (*n).into(),
        SqlValue::Real(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        SqlValue::Text(s) => serde_json::Value::String(s.clone()),
        SqlValue::Blob(_) => serde_json::Value::Null,
    }
}

fn val_to_sql_literal(v: &SqlValue) -> String {
    match v {
        SqlValue::Null => "NULL".to_string(),
        SqlValue::Integer(n) => n.to_string(),
        SqlValue::Real(f) => f.to_string(),
        SqlValue::Text(s) => format!("'{}'", s.replace('\'', "''")),
        SqlValue::Blob(b) => format!("X'{}'", to_hex(b)),
    }
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|byte| format!("{:02X}", byte)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use crate::{
        db::{query::OrderBy, schema::Column},
        filter::{predicate::filter_to_sql, ColumnFilter, Condition, FilterRule, FilterSet},
    };

    static NEXT_TEST_FILE_ID: AtomicUsize = AtomicUsize::new(0);

    fn column(name: &str, col_type: &str) -> Column {
        Column {
            name: name.to_string(),
            col_type: col_type.to_string(),
            not_null: false,
            default_value: None,
            is_pk: false,
            pk_position: 0,
            writable: true,
        }
    }

    fn temp_export_path(ext: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sqview-export-test-{}-{}.{}",
            process::id(),
            NEXT_TEST_FILE_ID.fetch_add(1, Ordering::Relaxed),
            ext
        ))
    }

    fn read_export(path: &Path) -> String {
        let content = fs::read_to_string(path).expect("read export");
        let _ = fs::remove_file(path);
        content
    }

    fn sorted_view(column: &str, ascending: bool, filter: &FilterSet) -> ViewQuery {
        let (where_clause, where_params) = filter_to_sql(filter).expect("compile filter");
        ViewQuery {
            order_by: vec![OrderBy {
                column: column.to_string(),
                ascending,
            }],
            where_clause,
            where_params,
            ..ViewQuery::table("items")
        }
    }

    fn literal_filter(column_name: &str, value: SqlValue) -> FilterSet {
        FilterSet {
            columns: HashMap::from([(
                column_name.to_string(),
                ColumnFilter {
                    rules: vec![FilterRule::new(Condition::Eq(value))],
                },
            )]),
        }
    }

    #[test]
    fn export_csv_applies_filter_sort_and_escaping() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE items (id INTEGER, category TEXT, note TEXT);
            INSERT INTO items (id, category, note) VALUES
              (2, 'keep', 'plain'),
              (1, 'drop', 'ignored'),
              (3, 'keep', 'say "hi",
again');
            "#,
        )
        .expect("seed items");

        let columns = vec![
            column("id", "INTEGER"),
            column("category", "TEXT"),
            column("note", "TEXT"),
        ];
        let filter = literal_filter("category", SqlValue::Text("keep".to_string()));
        let view = sorted_view("id", false, &filter);
        let path = temp_export_path("csv");

        let count = export(
            &conn,
            ExportFormat::Csv,
            &view.table,
            &columns,
            ExportRows::View(&view),
            &path,
        )
        .expect("export csv");
        let content = read_export(&path);
        let mut reader = csv::Reader::from_reader(content.as_bytes());

        let headers = reader.headers().expect("csv headers").clone();
        let rows: Vec<Vec<String>> = reader
            .records()
            .map(|record| {
                record
                    .expect("csv row")
                    .iter()
                    .map(str::to_string)
                    .collect()
            })
            .collect();

        assert_eq!(count, 2);
        assert_eq!(
            headers.iter().collect::<Vec<_>>(),
            vec!["id", "category", "note"]
        );
        assert_eq!(
            rows,
            vec![
                vec![
                    "3".to_string(),
                    "keep".to_string(),
                    "say \"hi\",\nagain".to_string(),
                ],
                vec!["2".to_string(), "keep".to_string(), "plain".to_string()],
            ]
        );
    }

    #[test]
    fn export_json_produces_parseable_strings_with_special_characters() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE items (id INTEGER, note TEXT, payload BLOB);
            INSERT INTO items (id, note, payload) VALUES
              (1, 'path C:\tmp\file "quoted"
next line', X'00FF');
            "#,
        )
        .expect("seed items");

        let columns = vec![
            column("id", "INTEGER"),
            column("note", "TEXT"),
            column("payload", "BLOB"),
        ];
        let view = sorted_view("id", true, &FilterSet::default());
        let path = temp_export_path("json");

        let count = export(
            &conn,
            ExportFormat::Json,
            &view.table,
            &columns,
            ExportRows::View(&view),
            &path,
        )
        .expect("export json");
        let content = read_export(&path);
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid json");

        assert_eq!(count, 1);
        assert_eq!(
            parsed,
            serde_json::json!([{
                "id": 1,
                "note": "path C:\\tmp\\file \"quoted\"\nnext line",
                "payload": null
            }])
        );
    }

    #[test]
    fn export_sql_escapes_text_and_blob_literals() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE items (id INTEGER, note TEXT, payload BLOB);
            INSERT INTO items (id, note, payload) VALUES
              (7, 'O''Reilly', X'00FF10');
            "#,
        )
        .expect("seed items");

        let columns = vec![
            column("id", "INTEGER"),
            column("note", "TEXT"),
            column("payload", "BLOB"),
        ];
        let path = temp_export_path("sql");

        let count = export(
            &conn,
            ExportFormat::Sql,
            "items",
            &columns,
            ExportRows::View(&ViewQuery::table("items")),
            &path,
        )
        .expect("export sql");
        let content = read_export(&path);

        assert_eq!(count, 1);
        assert_eq!(
            content,
            "INSERT INTO \"items\" (\"id\", \"note\", \"payload\") VALUES (7, 'O''Reilly', X'00FF10');\n"
        );
    }

    #[test]
    fn given_rows_become_clipboard_text() {
        let columns = vec![column("id", "INTEGER"), column("name", "TEXT")];
        let rows = vec![vec![SqlValue::Integer(1), SqlValue::Text("a,b".into())]];
        let csv = rows_to_text(ExportFormat::Csv, "t", &columns, &rows).expect("csv");
        assert_eq!(csv, "id,name\n1,\"a,b\"\n");
        let sql = rows_to_text(ExportFormat::Sql, "t", &columns, &rows).expect("sql");
        assert_eq!(
            sql,
            "INSERT INTO \"t\" (\"id\", \"name\") VALUES (1, 'a,b');\n"
        );
    }
}
