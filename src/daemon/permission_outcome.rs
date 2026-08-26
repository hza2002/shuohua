use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::platform::permissions::RuntimePermission;

const FILE_NAME: &str = "permission-required.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PermissionOutcome {
    pub permission: RuntimePermission,
    pub executable: PathBuf,
}

pub(crate) fn path() -> PathBuf {
    crate::paths::StateDirs::discover().root().join(FILE_NAME)
}

pub(crate) fn clear() -> Result<()> {
    clear_at(&path())
}

pub(crate) fn read() -> Result<Option<PermissionOutcome>> {
    read_at(&path())
}

pub(crate) fn write(permission: RuntimePermission) -> Result<()> {
    let outcome = PermissionOutcome {
        permission,
        executable: std::env::current_exe().context("resolve daemon executable")?,
    };
    write_at(&path(), &outcome)
}

fn clear_at(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

fn read_at(path: &Path) -> Result<Option<PermissionOutcome>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", path.display()))
        .map(Some)
}

fn write_at(path: &Path, outcome: &PermissionOutcome) -> Result<()> {
    let parent = path
        .parent()
        .context("permission outcome path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let temp = path.with_extension(format!("json.tmp-{}", ulid::Ulid::generate()));
    let body = serde_json::to_vec(outcome).context("encode permission outcome")?;
    let write_result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&body)?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp);
        return Err(error).with_context(|| format!("write {}", temp.display()));
    }
    fs::rename(&temp, path).with_context(|| format!("replace {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_round_trips_and_clear_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("shuo-permission-{}", ulid::Ulid::generate()));
        let path = dir.join(FILE_NAME);
        let outcome = PermissionOutcome {
            permission: RuntimePermission::Accessibility,
            executable: PathBuf::from("/Users/u/.local/bin/shuo"),
        };

        write_at(&path, &outcome).unwrap();
        assert_eq!(read_at(&path).unwrap(), Some(outcome));
        clear_at(&path).unwrap();
        clear_at(&path).unwrap();
        assert_eq!(read_at(&path).unwrap(), None);

        let _ = fs::remove_dir_all(dir);
    }
}
