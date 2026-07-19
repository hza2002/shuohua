use std::path::Path;
use std::sync::Arc;

/// 删除单个文件的策略。生产用 [`system_trash`]（移到系统废纸篓）；测试注入一个
/// 真实删除并记录路径的 deleter，绝不触碰真实 `~/.Trash`。失败以 `Err` 上报，调用方
/// 决定如何处理（本项目：记为错误并保留文件，绝不回退到永久删除）。
type DeleteFn = Arc<dyn Fn(&Path) -> anyhow::Result<()> + Send + Sync>;

#[derive(Clone)]
pub(crate) struct FileDeleter(DeleteFn);

impl FileDeleter {
    pub(crate) fn delete(&self, path: &Path) -> anyhow::Result<()> {
        (self.0)(path)
    }
}

impl std::fmt::Debug for FileDeleter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileDeleter")
    }
}

/// 通过 `trash` crate 把文件移到系统废纸篓。
pub(crate) fn system_trash() -> FileDeleter {
    FileDeleter(Arc::new(|path: &Path| {
        trash::delete(path).map_err(|e| anyhow::anyhow!("move {} to trash: {e}", path.display()))
    }))
}

/// 测试用 deleter：真实删除文件（不进废纸篓），并把被删路径记入 `log`，供断言
/// 「删除确实走了注入的 seam 而不是硬编码的 `fs::remove_file`」。测试绝不触碰真实
/// `~/.Trash`。
#[cfg(test)]
pub(crate) fn recording_deleter() -> (FileDeleter, Arc<std::sync::Mutex<Vec<std::path::PathBuf>>>) {
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = log.clone();
    let deleter = FileDeleter(Arc::new(move |path: &Path| {
        sink.lock().unwrap().push(path.to_path_buf());
        std::fs::remove_file(path).map_err(|e| anyhow::anyhow!("remove {}: {e}", path.display()))
    }));
    (deleter, log)
}

/// 测试用 deleter：真实删除文件（不进废纸篓）。给只需「别碰真实废纸篓」的测试用。
#[cfg(test)]
pub(crate) fn remove_deleter() -> FileDeleter {
    recording_deleter().0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_trash_errors_on_missing_path() {
        let deleter = system_trash();
        let missing =
            std::env::temp_dir().join(format!("shuohua-trash-missing-{}", ulid::Ulid::generate()));
        assert!(
            deleter.delete(&missing).is_err(),
            "trashing a nonexistent path must surface an error"
        );
    }
}
