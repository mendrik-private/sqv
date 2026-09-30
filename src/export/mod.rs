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

/// Streams every row of `view` into `path`, replacing it atomically, and returns
/// the number of exported rows.
pub fn export(
    conn: &Connection,
    format: ExportFormat,
    view: &ViewQuery,
    columns: &[Column],
    path: &Path,
) -> anyhow::Result<u64> {
    atomic_export(path, |file| match format {
        ExportFormat::Csv => write_csv(conn, view, columns, file),
        ExportFormat::Json => write_json(conn, view, columns, file),
        ExportFormat::Sql => write_sql(conn, view, columns, file),
    })
}

fn write_csv(
    conn: &Connection,
    view: &ViewQuery,
    columns: &[Column],
    file: File,
) -> anyhow::Result<u64> {
    let mut writer = csv::Writer::from_writer(file);
    writer.write_record(columns.iter().map(|column| column.name.as_str()))?;
    let count = visit_rows(conn, view, columns, |row| {
        writer.write_record(row.iter().map(val_to_str))?;
        Ok(())
    })?;
    writer.flush()?;
    Ok(count)
}

fn write_json(
    conn: &Connection,
    view: &ViewQuery,
    columns: &[Column],
    file: File,
) -> anyhow::Result<u64> {
    let mut out = BufWriter::new(file);
    out.write_all(b"[")?;
    let mut first = true;
    let count = visit_rows(conn, view, columns, |row| {
        if !first {
            out.write_all(b",")?;
        }
        first = false;
        serde_json::to_writer(&mut out, &row_to_json(columns, &row))?;
        Ok(())
    })?;
    out.write_all(b"]\n")?;
    out.flush()?;
    Ok(count)
}

fn write_sql(
    conn: &Connection,
    view: &ViewQuery,
    columns: &[Column],
    file: File,
) -> anyhow::Result<u64> {
    let col_names = columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let quoted_table = view.quoted_table();
    let mut out = BufWriter::new(file);
    let count = visit_rows(conn, view, columns, |row| {
        let values = row.iter().map(val_to_sql_literal).collect::<Vec<_>>();
        writeln!(
            out,
            "INSERT INTO {} ({}) VALUES ({});",
            quoted_table,
            col_names,
            values.join(", ")
        )?;
        Ok(())
    })?;
    out.flush()?;
    Ok(count)
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
            order_by: Some(OrderBy {
                column: column.to_string(),
                ascending,
            }),
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

        let count = export(&conn, ExportFormat::Csv, &view, &columns, &path).expect("export csv");
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

        let count = export(&conn, ExportFormat::Json, &view, &columns, &path).expect("export json");
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
            &ViewQuery::table("items"),
            &columns,
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
}
