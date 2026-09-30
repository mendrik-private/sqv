use directories::ProjectDirs;
use std::path::PathBuf;

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "sqview")
}

pub fn config_file() -> Option<PathBuf> {
    Some(project_dirs()?.config_dir().join("config.toml"))
}

pub fn data_local_dir() -> Option<PathBuf> {
    Some(project_dirs()?.data_local_dir().to_path_buf())
}

/// Saved per-table view settings (filters, sort, columns).
pub fn view_settings_dir() -> Option<PathBuf> {
    Some(data_local_dir()?.join("views"))
}

/// SQL console history.
pub fn history_file() -> Option<PathBuf> {
    Some(data_local_dir()?.join("sql_history"))
}
