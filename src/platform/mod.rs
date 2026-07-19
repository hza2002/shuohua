pub(crate) mod autotype;
pub(crate) mod clipboard;
pub(crate) mod daemon;
#[cfg(target_os = "macos")]
pub mod macos;
pub(crate) mod permissions;

/// 前台 App 上下文。daemon 在 toggle OFF 时取一次，整条 pipeline 共享。
#[derive(Debug, Default, Clone)]
pub struct AppContext {
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
}
