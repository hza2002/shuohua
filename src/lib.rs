//! Reusable client-side surface for external shuohua clients.
//!
//! The binary owns daemon/runtime/UI/platform modules. This library intentionally
//! exposes only the existing daemon client API, IPC protocol/transport, and DTOs
//! needed by that protocol.

pub mod client_api;
pub mod history;
pub mod ipc;
pub mod paths;
pub mod state;
pub mod text_stats;
