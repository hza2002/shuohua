mod command;
pub mod layout;
mod model;

pub use command::{OverlayCmd, OverlayHandle, OverlayState, TextKind};
pub use model::OverlayModel;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::run;

// Transitional shim: 让旧 `overlay::view::run` 路径继续工作，避免和并行工作冲突。
// Task 7 文档收尾时一并迁移 main.rs 调用点后删除。
#[cfg(target_os = "macos")]
pub mod view {
    pub use super::macos::run;
}
