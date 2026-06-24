use std::path::PathBuf;

pub fn config_home() -> PathBuf {
    config_root()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn config_root() -> PathBuf {
    crate::paths::AppPaths::discover()
        .config_root()
        .to_path_buf()
}

pub fn main_config() -> PathBuf {
    crate::paths::AppPaths::discover().main_config()
}

pub fn profile_dir() -> PathBuf {
    crate::paths::AppPaths::discover().profile_dir()
}

pub fn asr_provider(provider: &str) -> PathBuf {
    crate::paths::AppPaths::discover().asr_provider(provider)
}

pub fn post_dir() -> PathBuf {
    crate::paths::AppPaths::discover().post_dir()
}
