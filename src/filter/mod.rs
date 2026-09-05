pub mod predicate;
pub mod rule;

pub use rule::{ColumnFilter, FilterOp, FilterSet, FilterValue};

use std::path::PathBuf;

pub fn filter_path(db_path: &str, table_name: &str) -> Option<PathBuf> {
    let database_identity = if db_path == ":memory:" {
        db_path.as_bytes().to_vec()
    } else {
        std::fs::canonicalize(db_path)
            .unwrap_or_else(|_| std::path::PathBuf::from(db_path))
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
        crate::app_dirs::data_local_dir()?
            .join("filters-v2")
            .join(format!("{database_key:016x}"))
            .join(format!("{table_key}.toml")),
    )
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

pub fn save_filter(filter: &FilterSet, db_path: &str, table_name: &str) -> anyhow::Result<()> {
    let path = filter_path(db_path, table_name)
        .ok_or_else(|| anyhow::anyhow!("could not determine filter path"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string(filter)?;
    let temporary = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&temporary, content)?;
    std::fs::rename(temporary, &path)?;
    Ok(())
}

pub fn load_filter(db_path: &str, table_name: &str) -> anyhow::Result<FilterSet> {
    let path = filter_path(db_path, table_name)
        .ok_or_else(|| anyhow::anyhow!("could not determine filter path"))?;
    let content = std::fs::read_to_string(&path)?;
    let filter: FilterSet = toml::from_str(&content)?;
    Ok(filter)
}

#[cfg(test)]
mod tests {
    use super::filter_path;

    #[test]
    fn table_name_cannot_escape_filter_directory() {
        let normal = filter_path("/tmp/example.db", "items").expect("filter path");
        let hostile = filter_path("/tmp/example.db", "/../../outside").expect("filter path");
        assert_eq!(normal.parent(), hostile.parent());
        assert_ne!(normal.file_name(), hostile.file_name());
    }

    #[test]
    fn databases_with_the_same_filename_have_distinct_directories() {
        let first = filter_path("/tmp/one/data.db", "items").expect("filter path");
        let second = filter_path("/tmp/two/data.db", "items").expect("filter path");
        assert_ne!(first.parent(), second.parent());
    }
}
