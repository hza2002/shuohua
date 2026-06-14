//! Post pipeline 需要的前台 App 上下文入口。
//!
//! macOS 实现复用 crate 顶层已有的 AppKit bridge。这个模块是 post 层对外的
//! 稳定边界，避免 processor 直接依赖 Darwin 细节。

pub use super::AppContext;

#[cfg(target_os = "macos")]
pub fn frontmost_app() -> AppContext {
    crate::app_context_darwin::frontmost_app()
}

#[cfg(not(target_os = "macos"))]
pub fn frontmost_app() -> AppContext {
    AppContext::default()
}
