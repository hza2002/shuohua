use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDirs {
    root: PathBuf,
}

impl StateDirs {
    pub fn discover() -> Self {
        if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
            return Self::from_root(PathBuf::from(xdg).join("shuohua"));
        }
        Self::from_root(
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state/shuohua"),
        )
    }

    pub fn from_root(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn history(&self) -> PathBuf {
        self.root.join("history")
    }

    pub fn audio(&self) -> PathBuf {
        self.root.join("audio")
    }

    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    #[cfg_attr(not(feature = "dev"), allow(dead_code))]
    pub fn traces(&self) -> PathBuf {
        self.root.join("traces")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_subdirectories_share_one_root() {
        let root = PathBuf::from("/tmp/shuohua-state");
        let dirs = StateDirs::from_root(root.clone());
        assert_eq!(dirs.root(), root);
        assert_eq!(dirs.history(), root.join("history"));
        assert_eq!(dirs.audio(), root.join("audio"));
        assert_eq!(dirs.logs(), root.join("logs"));
        assert_eq!(dirs.traces(), root.join("traces"));
    }
}
