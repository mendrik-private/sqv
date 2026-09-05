use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rusqlite::Connection;

use crate::{
    db::{schema::Column, types::SqlValue},
    filter::{predicate::filter_to_sql, FilterSet},
    grid::{SortDir, SortSpec},
};

pub fn export_csv(
    conn: &Connection,
    table: &str,
    columns: &[Column],
    filter: &FilterSet,
    sort: &Option<SortSpec>,
    path: &Path,
) -> anyhow::Result<u64> {
    let (where_clause, where_params) = filter_to_sql(filter)?;
    let order = sort.as_ref().and_then(|s| {
        columns
            .get(s.col_idx)
            .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
    });
    let order_ref = order.as_ref().map(|(s, b)| (s.as_str(), *b));

    atomic_export(path, |file| {
        let mut writer = csv::Writer::from_writer(file);
        writer.write_record(columns.iter().map(|column| column.name.as_str()))?;
        let count = crate::db::visit_rows(
            conn,
            row_fetch(table, columns, order_ref, &where_clause, &where_params),
            |row| {
                writer.write_record(row.iter().map(val_to_str))?;
                Ok(())
            },
        )?;
        writer.flush()?;
        Ok(count)
    })
}

pub fn export_json(
    conn: &Connection,
    table: &str,
    columns: &[Column],
    filter: &FilterSet,
    sort: &Option<SortSpec>,
    path: &Path,
) -> anyhow::Result<u64> {
    let (where_clause, where_params) = filter_to_sql(filter)?;
    let order = sort.as_ref().and_then(|s| {
        columns
            .get(s.col_idx)
            .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
    });
    let order_ref = order.as_ref().map(|(s, b)| (s.as_str(), *b));

    atomic_export(path, |file| {
        let mut out = BufWriter::new(file);
        out.write_all(b"[")?;
        let mut first = true;
        let count = crate::db::visit_rows(
            conn,
            row_fetch(table, columns, order_ref, &where_clause, &where_params),
            |row| {
                if !first {
                    out.write_all(b",")?;
                }
                first = false;
                let object = columns
                    .iter()
                    .zip(row)
                    .map(|(column, value)| (column.name.clone(), val_to_json(value)))
                    .collect::<serde_json::Map<_, _>>();
                serde_json::to_writer(&mut out, &object)?;
                Ok(())
            },
        )?;
        out.write_all(b"]\n")?;
        out.flush()?;
        Ok(count)
    })
}

pub fn export_sql(
    conn: &Connection,
    table: &str,
    columns: &[Column],
    filter: &FilterSet,
    sort: &Option<SortSpec>,
    path: &Path,
) -> anyhow::Result<u64> {
    let (where_clause, where_params) = filter_to_sql(filter)?;
    let order = sort.as_ref().and_then(|s| {
        columns
            .get(s.col_idx)
            .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
    });
    let order_ref = order.as_ref().map(|(s, b)| (s.as_str(), *b));

    let col_names: String = columns
        .iter()
        .map(|column| crate::db::query::quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let quoted_table = crate::db::query::quote_identifier(table);
    atomic_export(path, |file| {
        let mut out = BufWriter::new(file);
        let count = crate::db::visit_rows(
            conn,
            row_fetch(table, columns, order_ref, &where_clause, &where_params),
            |row| {
                let values = row.iter().map(val_to_sql_literal).collect::<Vec<_>>();
                writeln!(
                    out,
                    "INSERT INTO {} ({}) VALUES ({});",
                    quoted_table,
                    col_names,
                    values.join(", ")
                )?;
                Ok(())
            },
        )?;
        out.flush()?;
        Ok(count)
    })
}

fn row_fetch<'a>(
    table: &'a str,
    columns: &'a [Column],
    order_by: Option<(&'a str, bool)>,
    where_clause: &'a str,
    where_params: &'a [rusqlite::types::Value],
) -> crate::db::RowFetch<'a> {
    crate::db::RowFetch {
        table,
        columns,
        offset: 0,
        limit: i64::MAX,
        order_by,
        where_clause,
        where_params,
    }
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

fn val_to_str(v: &SqlValue) -> String {
    match v {
        SqlValue::Null => String::new(),
        SqlValue::Integer(n) => n.to_string(),
        SqlValue::Real(f) => f.to_string(),
        SqlValue::Text(s) => s.clone(),
        SqlValue::Blob(b) => format!("<blob {} bytes>", b.len()),
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
        db::schema::Column,
        filter::{rule::FilterRule, ColumnFilter, FilterOp, FilterSet, FilterValue},
    };

    static NEXT_TEST_FILE_ID: AtomicUsize = AtomicUsize::new(0);

    fn column(cid: i64, name: &str, col_type: &str) -> Column {
        Column {
            cid,
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

    fn literal_filter(column_name: &str, value: SqlValue) -> FilterSet {
        FilterSet {
            columns: HashMap::from([(
                column_name.to_string(),
                ColumnFilter {
                    rules: vec![FilterRule {
                        op: FilterOp::Eq,
                        value: FilterValue::Literal(value),
                        enabled: true,
                        label: None,
                    }],
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
            column(0, "id", "INTEGER"),
            column(1, "category", "TEXT"),
            column(2, "note", "TEXT"),
        ];
        let filter = literal_filter("category", SqlValue::Text("keep".to_string()));
        let sort = Some(SortSpec {
            col_idx: 0,
            direction: SortDir::Desc,
        });
        let path = temp_export_path("csv");

        let count =
            export_csv(&conn, "items", &columns, &filter, &sort, &path).expect("export csv");
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
            column(0, "id", "INTEGER"),
            column(1, "note", "TEXT"),
            column(2, "payload", "BLOB"),
        ];
        let filter = FilterSet::default();
        let sort = Some(SortSpec {
            col_idx: 0,
            direction: SortDir::Asc,
        });
        let path = temp_export_path("json");

        let count =
            export_json(&conn, "items", &columns, &filter, &sort, &path).expect("export json");
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
            column(0, "id", "INTEGER"),
            column(1, "note", "TEXT"),
            column(2, "payload", "BLOB"),
        ];
        let path = temp_export_path("sql");

        let count = export_sql(
            &conn,
            "items",
            &columns,
            &FilterSet::default(),
            &None,
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
