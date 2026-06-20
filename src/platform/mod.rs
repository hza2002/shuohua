pub(crate) mod autotype;
pub(crate) mod clipboard;
pub(crate) mod daemon;
#[cfg(target_os = "macos")]
pub mod macos;
pub(crate) mod permissions;
