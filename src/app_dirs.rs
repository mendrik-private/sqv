use directories::ProjectDirs;
use std::path::PathBuf;

const APP_NAME: &str = "sqview";
const LEGACY_APP_NAME: &str = "sqv";

pub fn current_config_dir() -> Option<PathBuf> {
    Some(
        ProjectDirs::from("", "", APP_NAME)?
            .config_dir()
            .to_path_buf(),
    )
}

pub fn legacy_config_dir() -> Option<PathBuf> {
    Some(
        ProjectDirs::from("", "", LEGACY_APP_NAME)?
            .config_dir()
            .to_path_buf(),
    )
}

pub fn config_dir() -> Option<PathBuf> {
    let current = current_config_dir()?;
    let legacy = legacy_config_dir()?;

    if current.exists() {
        Some(current)
    } else if legacy.exists() {
        Some(legacy)
    } else {
        current_config_dir()
    }
}

pub fn config_file() -> Option<PathBuf> {
    Some(config_dir()?.join("config.toml"))
}

pub fn current_config_file() -> Option<PathBuf> {
    Some(current_config_dir()?.join("config.toml"))
}

pub fn legacy_config_file() -> Option<PathBuf> {
    Some(legacy_config_dir()?.join("config.toml"))
}

pub fn data_local_dir() -> Option<PathBuf> {
    let current = ProjectDirs::from("", "", APP_NAME)?;
    let legacy = ProjectDirs::from("", "", LEGACY_APP_NAME)?;

    if current.data_local_dir().exists() {
        Some(current.data_local_dir().to_path_buf())
    } else if legacy.data_local_dir().exists() {
        Some(legacy.data_local_dir().to_path_buf())
    } else {
        Some(current.data_local_dir().to_path_buf())
    }
}

pub fn filter_dir() -> Option<PathBuf> {
    Some(data_local_dir()?.join("filters"))
}
