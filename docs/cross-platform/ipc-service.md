# IPC And Service

## IPC

JSON-line command/event protocol 保持共享。平台只替换 transport：

| 平台 | Transport |
|---|---|
| macOS | Unix domain socket |
| Linux | Unix domain socket |
| Windows | Named Pipe |

协议层不应依赖 transport path 类型。client/server 应通过 transport facade 创建 stream。

Phase 3 先落地最小 transport facade，不改变协议、不改变 daemon 生命周期：

- `src/ipc/protocol.rs` 只保留 JSON-line encode/decode 和 wire model，不 import transport/path。
- `src/ipc/transport.rs` 拥有 endpoint 类型、默认 endpoint、connect、bind、accept 和 stale endpoint
  清理。
- `src/ipc/client.rs` 只处理 command/event 读写，不直接 import `tokio::net::UnixStream` 或
  Windows Named Pipe 类型。
- `src/ipc/server.rs` 只处理 command dispatch、state/history fanout 和 shutdown reply，不直接
  import `tokio::net::UnixListener` / `UnixStream`。
- macOS 和 Linux Phase 3 继续使用 Unix domain socket；Windows 只保留接口形状，具体 Named Pipe
  backend 后续实现。

Transport facade 可以先暴露 concrete stream/listener wrapper，不急于引入 async trait 或动态分发。
只有当 Windows backend 落地时，再决定是否需要 enum backend、trait object 或条件编译 sibling。

## 单实例

daemon 单实例锁和 stale endpoint 清理需要平台化：

- macOS/Linux：lock file + UDS endpoint。
- Windows：named mutex 或 lock file + Named Pipe endpoint。

stale endpoint 清理仍只允许 daemon 持 lock 的启动路径执行。

Phase 3 不抽 daemon lock。现有启动顺序已经先持有 `DaemonLock` 再 bind IPC endpoint；因此
UDS stale cleanup 可暂时留在 `ipc::transport::bind_default()` 内，但调用方必须仍只从 daemon
启动路径进入。Phase 4 再把 lock、process probe 和 service manager 一起平台化。

## Service Manager

Service manager 是用户会话级服务，不按 server daemon 设计：

| 平台 | 推荐 |
|---|---|
| macOS | launchd user agent |
| Linux | systemd user |
| Windows | logon task / startup app；慎用 Windows Service |

Windows Service 通常不适合需要用户会话、桌面权限、clipboard/text injection 的应用。

## Smart Fallback

CLI/TUI/GUI 连接 daemon 时：

1. probe endpoint。
2. 如果不存在，按平台策略尝试启动用户 daemon。
3. 等待 ready event 或超时。
4. 如果 endpoint stale，只允许 daemon lock owner 清理。

## 验收

- macOS 当前 JSON-line 协议不变。
- TUI/GUI/client 不关心 transport 细节。
- Windows backend 只需替换 transport，不 fork protocol。
- service status 输出使用相同 user-facing contract。
- `cargo test --test platform_layout` 保护 protocol/client/server 和 transport 的 import 边界。
