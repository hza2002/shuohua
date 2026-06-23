//! Shared daemon client API for TUI and the future GUI backend.
//!
//! This layer intentionally stays thin: it owns no wire protocol and only
//! exposes existing IPC commands/events through a stable client boundary.

use crate::ipc::protocol::Command;

pub(crate) type DaemonClient = crate::ipc::client::IpcClient;

pub(crate) fn subscribe_command() -> Command {
    Command::Subscribe
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::protocol::PROTO_VERSION;

    #[test]
    fn shared_client_boundary_uses_existing_ipc_protocol() {
        assert_eq!(PROTO_VERSION, 2);
        assert_eq!(subscribe_command(), Command::Subscribe);
    }
}
