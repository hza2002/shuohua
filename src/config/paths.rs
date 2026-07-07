use std::path::PathBuf;

pub fn config_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
}

pub fn root_dir() -> PathBuf {
    config_home().join("shuohua")
}

pub fn main_config() -> PathBuf {
    root_dir().join("config.toml")
}

pub fn profile_dir() -> PathBuf {
    root_dir().join("profile")
}

pub fn post_dir() -> PathBuf {
    root_dir().join("post")
}
