//! How the user last viewed each table: filters, sort keys, hidden columns,
//! column widths and the frozen first column, saved per database and table.

use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use crate::filter::FilterSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortKey {
    pub column: String,
    pub ascending: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewSettings {
    pub filter: FilterSet,
    pub sort: Vec<SortKey>,
    pub hidden: BTreeSet<String>,
    pub widths: HashMap<String, u16>,
    pub frozen: bool,
}

impl ViewSettings {
    pub fn is_default(&self) -> bool {
        self.filter.is_empty()
            && self.sort.is_empty()
            && self.hidden.is_empty()
            && self.widths.is_empty()
            && !self.frozen
    }
}

/// Where the settings of `table_name` in the database at `db_path` live. The
/// database path is hashed and the table name hex-encoded, so no name can
/// escape the settings directory or collide with another database's tables.
pub fn settings_path(db_path: &str, table_name: &str) -> Option<PathBuf> {
    let database_identity = if db_path == IN_MEMORY {
        db_path.as_bytes().to_vec()
    } else {
        std::fs::canonicalize(db_path)
            .unwrap_or_else(|_| PathBuf::from(db_path))
            .to_string_lossy()
            .as_bytes()
            .to_vec()
    };
    let database_key = fnv1a64(&database_identity);
    let table_key = table_name
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Some(
        crate::app_dirs::view_settings_dir()?
            .join(format!("{database_key:016x}"))
            .join(format!("{table_key}.toml")),
    )
}

const IN_MEMORY: &str = ":memory:";

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

/// Saves the settings of a table. An in-memory database is new every time,
/// so its settings are not kept.
pub fn save(settings: &ViewSettings, db_path: &str, table_name: &str) -> anyhow::Result<()> {
    if db_path == IN_MEMORY {
        return Ok(());
    }
    let path = settings_path(db_path, table_name)
        .ok_or_else(|| anyhow::anyhow!("could not determine the settings path"))?;
    if settings.is_default() {
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => return Ok(()),
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string(settings)?;
    let temporary = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&temporary, content)?;
    std::fs::rename(temporary, &path)?;
    Ok(())
}

/// The saved settings, or defaults when none are saved or they cannot be read.
pub fn load(db_path: &str, table_name: &str) -> ViewSettings {
    if db_path == IN_MEMORY {
        return ViewSettings::default();
    }
    settings_path(db_path, table_name)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::types::SqlValue,
        filter::{ColumnFilter, Condition, FilterRule},
    };

    #[test]
    fn table_name_cannot_escape_the_settings_directory() {
        let normal = settings_path("/tmp/example.db", "items").expect("path");
        let hostile = settings_path("/tmp/example.db", "/../../outside").expect("path");
        assert_eq!(normal.parent(), hostile.parent());
        assert_ne!(normal.file_name(), hostile.file_name());
    }

    #[test]
    fn databases_with_the_same_filename_have_distinct_directories() {
        let first = settings_path("/tmp/one/data.db", "items").expect("path");
        let second = settings_path("/tmp/two/data.db", "items").expect("path");
        assert_ne!(first.parent(), second.parent());
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let mut settings = ViewSettings {
            sort: vec![SortKey {
                column: "name".into(),
                ascending: false,
            }],
            frozen: true,
            ..ViewSettings::default()
        };
        settings.hidden.insert("notes".into());
        settings.widths.insert("name".into(), 30);
        settings.filter.columns.insert(
            "qty".into(),
            ColumnFilter {
                rules: vec![FilterRule::new(Condition::Gt(SqlValue::Integer(3)))],
            },
        );
        let text = toml::to_string(&settings).expect("serialize");
        assert_eq!(
            toml::from_str::<ViewSettings>(&text).expect("parse"),
            settings
        );
    }
}
