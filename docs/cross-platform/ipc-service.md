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

Phase 3b implementation status:

- `src/ipc/transport.rs` 已拆成 cfg-gated backend module。Unix backend 保持现有 UDS endpoint、
  stale socket 清理和 0600 权限语义；Windows backend 只是 Named Pipe placeholder，返回
  structured unsupported error。
- `platform::lifecycle` 已同步拆成 Unix backend 和 Windows unsupported skeleton，避免 Windows target
  编译到 `flock`、`kill(pid, 0)` 和 Unix file metadata API。
- `daemon::fallback` 的 endpoint probe 已 cfg-gate：Unix 仍用同步 UDS connect；Windows 当前保守视为
  endpoint absent，后续 Named Pipe probe 阶段再替换。
- 这一步只解除 Windows target 被 Unix-only IPC/lifecycle API 直接阻断的问题，不实现 Named Pipe、
  Windows daemon lock、Windows process probing 或 smart fallback 启动策略。

Phase 3c Windows Named Pipe transport compile backend:

- Windows `ipc::transport` 使用 Tokio `tokio::net::windows::named_pipe`，保持 JSON-line
  command/event protocol 不变。
- Server `accept()` 在当前 pipe instance 连接后创建下一条 pipe instance，再把已连接 stream
  交给 IPC server，避免短窗口内 client 看到 pipe not found。
- Client `connect()` 只处理 transport connect；遇到 pipe busy 做短退避重试，不启动 daemon、
  不 probe service、不改变 smart fallback 策略。
- 该阶段只要求 Windows target compile；Named Pipe ACL、安全描述符、multi-user 隔离、
  stale endpoint 判定和真实 Windows runtime 行为仍需 Windows 实机/VM 验证。

Phase 10r Windows Named Pipe endpoint scoping/security descriptor hardening:

- Windows `default_endpoint()` 不再使用固定 `\\.\pipe\shuohua`，而是使用当前 user SID +
  logon SID 的 SHA-256 prefix 生成 `\\.\pipe\shuohua-<scope>`。
- Windows daemon mutex 使用相同 scope suffix：`Local\shuohua-daemon-<scope>`。
- Server pipe instance 创建时传入显式 security descriptor。当前 DACL 只授予 current user SID、
  LocalSystem 和 Built-in Administrators，不授予 Everyone/World 或 Anonymous；不再依赖默认
  Named Pipe security descriptor。
- 已在当前 Windows session 验证：daemon 能启动、`service status` 能通过 Named Pipe 取得
  `DaemonStatus`，第二个 daemon 被 named mutex 拒绝。
- 后续阶段已完成 elevated/non-elevated 行为矩阵、busy-pipe smoke 和 client access mask 收窄。因此
  capability reason 已收窄为 `same_user_elevation_smoke_only`，但 status 仍保持 `partial`。
- 仍未完成：cross-user 验证和长时间 runtime soak；完成前不得把 Windows IPC capability 升级为
  `available`。

Phase 10t Windows Named Pipe busy retry policy:

- Client connect 的 `ERROR_PIPE_BUSY` retry 仍是 bounded short retry：最多 20 次 open attempt，
  每次 busy 后等待 50ms。该策略只避免短暂 server instance 切换窗口内立即失败，不启动 daemon，
  不进入 smart fallback。
- 当前已补单元测试固定 retry 边界；尚未做真实 busy-pipe 压力测试或高并发 client runtime soak。

Phase 10x Windows Named Pipe client access-mask audit:

- 当前 client connect 仍使用 Tokio `ClientOptions::new().open(...)`。Tokio 1.52 的公开选项只允许
  `read`/`write` 布尔配置，并映射到 Windows `GENERIC_READ` / `GENERIC_WRITE`；没有公开 API 传入
  更窄的 desired access mask。
- 因此 Phase 10r/10w 已完成的是 endpoint scope、server-side DACL、mutex security descriptor 和
  elevation split 修复，不代表 client access mask 已收窄。
- 真正收窄 client mask 需要后续单独实现 raw `CreateFileW` + overlapped handle -> Tokio pipe client
  的路径，或等待/引入支持 explicit desired access 的 Tokio API；这一步必须重新做 Windows runtime
  smoke，尤其是 client connect、busy retry、cross-elevation 和 cross-user 行为。

Phase 10af Windows raw Named Pipe client access mask:

- Windows client connect 改为 raw `CreateFileW` + `NamedPipeClient::from_raw_handle`，不再通过
  `ClientOptions::new().open(...)` 取得 `GENERIC_READ` / `GENERIC_WRITE`。
- Desired access 只包含 JSON-line client 需要的 `FILE_READ_DATA | FILE_WRITE_DATA`，并保留
  `FILE_FLAG_OVERLAPPED` 和 `SECURITY_IDENTIFICATION | SECURITY_SQOS_PRESENT`。
- `ERROR_PIPE_BUSY` 仍使用既有 bounded retry policy；access/scope/security 错误仍保持可见，不触发
  service install/startup registration。
- 该阶段必须在 Windows 重新跑 daemon/status/busy/service lifecycle smoke；cross-user 第二账号/VM
  仍是 deferred manual gate。

Phase 10bc Windows IPC/lifecycle capability diagnostics:

- Windows `ipc.transport` 和 `daemon.single_instance` capability reason 从旧的 `runtime_not_verified`
  收窄为 `same_user_elevation_smoke_only`，表达 same-user、elevated/non-elevated、busy clients 和
  raw access-mask smoke 已通过。
- Windows `process.probe` reason 收窄为 `service_lifecycle_smoke_only`，因为 `service stop` /
  `restart` 已通过 `OpenProcess` probe 做 bounded exit wait。
- 这只是诊断真实性更新，不改变 IPC/service/lifecycle 代码，不升级 capability；cross-user 和 longer
  soak 仍是 deferred manual gate。

## 单实例

daemon 单实例锁和 stale endpoint 清理需要平台化：

- macOS/Linux：lock file + UDS endpoint。
- Windows：named mutex 或 lock file + Named Pipe endpoint。

stale endpoint 清理仍只允许 daemon 持 lock 的启动路径执行。

Phase 3 不抽 daemon lock。现有启动顺序已经先持有 `DaemonLock` 再 bind IPC endpoint；因此
UDS stale cleanup 可暂时留在 `ipc::transport::bind_default()` 内，但调用方必须仍只从 daemon
启动路径进入。

Phase 4a 先抽两个低风险 facade：

- `platform::lifecycle::acquire_daemon_lock()`：保持 macOS 当前 lock file + `flock` 语义。
- `platform::lifecycle::process_exists(pid)`：保持 macOS 当前 `kill(pid, 0)` 语义，
  `EPERM` 视为进程仍存在，`ESRCH` 视为已退出。

`daemon::process` 和 `cli::service::macos` 不再直接依赖 Unix lock/process primitive。
stale endpoint cleanup 仍只通过 daemon 持 lock 后的 bind 路径发生。

Phase 4b 再抽 service manager facade；不要和 lock/process probe 混在同一 commit。

## Service Manager

Service manager 是用户会话级服务，不按 server daemon 设计：

| 平台 | 推荐 |
|---|---|
| macOS | launchd user agent |
| Linux | systemd user |
| Windows | logon task / startup app；慎用 Windows Service |

Windows Service 通常不适合需要用户会话、桌面权限、clipboard/text injection 的应用。

Phase 4a 不改 `shuo app service` 用户可见语义，也不搬 launchd 实现。Phase 4b 的目标是让
`cli::service` 依赖一个 service manager facade，同时保持 macOS launchd 输出、timeout、
stop/restart 顺序不变。

Phase 4b 的 facade 边界：

- `platform::service` 拥有用户会话级 service manager backend 选择。
- macOS backend 继续使用 launchd user agent，plist path、label、`launchctl` 调用、status
  输出、stop timeout 和 restart 顺序保持不变。
- `cli::service` 只保留 clap command、命令分发和向后兼容的 `launchd_status()` 入口，不直接
  拥有 `launchctl`、plist 生成、平台 unsupported 文案或具体 service manager backend。
- 非 macOS 继续返回明确 unsupported；Phase 4b 不实现 systemd user 或 Windows logon task。
- `doctor` 暂时可以继续调用 `cli::service::launchd_status()` 兼容入口，后续诊断阶段再改成通用
  service status 模型。

Phase 10e Linux systemd user dry-run/status skeleton:

- Linux backend 可以生成 systemd user unit path 和 unit body，但不得写入文件、不得调用
  `systemctl --user`、不得 enable/start/stop daemon。
- `shuo service status` 在 Linux 上可以打印 daemon IPC status（如果能连上）和 systemd user unit
  dry-run 信息：unit path、unit name、install/start unsupported。
- `install` / `uninstall` / `start` / `stop` / `restart` 仍返回明确 unsupported。Phase 10e 不改变
  CLI command shape，不新增 `--dry-run` 参数。
- systemd unit baseline 使用当前 executable + `--daemon`，`Restart=on-failure`，
  `RestartSec=2s`。真实 install 阶段再决定 XDG/systemd 目录创建、reload、enable、linger 和日志策略。
- macOS launchd backend 的 plist、timeout、stop/restart/status 语义不得改变。

Phase 10j Windows lifecycle primitive compile backend:

- `platform::lifecycle` 的 Windows backend 使用 Win32 named mutex 表达 daemon single-instance
  guard，使用 `OpenProcess` 表达 process probe。
- 这只解除 Windows target 在 lifecycle primitive 上的 pure placeholder 状态。命名空间、ACL/security
  descriptor、abandoned mutex、PID reuse、权限差异和多用户隔离仍需 Windows VM/实机验证。
- Phase 10j 不实现 Windows service manager、smart fallback、daemon auto-start 或 Named Pipe ACL。

Phase 10k Windows service manager dry-run/status skeleton:

- Windows backend 可以打印未来 user-session service/logon-task 策略的 dry-run status，但不得创建、
  注册、启动或停止任何服务。
- `install` / `uninstall` / `start` / `stop` / `restart` 仍返回明确 unsupported。
- Phase 10k 不调用 Task Scheduler、SCM、PowerShell 或 registry APIs，不写文件，不实现 smart fallback。

Phase 10ac Windows service stop IPC shutdown:

- Windows `shuo service stop` 可以连接当前用户/登录会话的 Named Pipe，发送既有 `Command::Shutdown`，
  收到 `DaemonStatus` 后用 `OpenProcess` probe 做有界等待。
- 这只是停止已经运行的 user-session daemon；不安装、不注册、不启动 daemon，也不调用 Task Scheduler、
  SCM、PowerShell 或 registry APIs。
- `install` / `uninstall` / `start` / `restart` 仍返回明确 unsupported；`service.manager` capability
  仍保持 `partial`，不能据此声明 Windows service lifecycle runtime-ready。

Phase 10ad Windows smart fallback Named Pipe probe:

- Windows `run_smart_fallback()` 不再把 endpoint 永远视为 absent；它用现有 Named Pipe transport
  probe 当前 scoped endpoint，能区分 pipe-not-found、pipe-busy/present 和 access/scope 类错误。
- Absent endpoint 仍只启动当前 executable 的 `--daemon` 子进程并等待 Named Pipe ready；不调用
  Task Scheduler、SCM、PowerShell、registry，也不实现 service install/start。
- 该路径只服务无参数 `shuo.exe` 进入 TUI 前的 developer/runtime convenience；Windows IPC capability
  仍不得在 cross-user 和更长 soak 前升级为 `available`。

Phase 10ae Windows user-session service start/restart:

- Windows `shuo service start` 可以显式启动当前 executable 的 `--daemon` 子进程，并等待当前用户/
  登录会话 scoped Named Pipe 返回 `DaemonStatus`。
- 如果 daemon 已运行，`start` 只报告当前 daemon status 和 already-running 信息，不生成第二个 daemon。
- Windows `shuo service restart` 先查询当前 daemon；运行中则走既有 IPC `Shutdown` + PID exit wait，
  然后再执行 `start`。daemon absent 时，`restart` 退化为 explicit start。
- 这仍不是 install/auto-start registration：`install` / `uninstall` 继续 unsupported，不调用
  Task Scheduler、SCM、PowerShell 或 registry APIs。

Phase 10be Windows startup registration error boundary:

- Windows `shuo service install` / `uninstall` 仍不创建或删除 Task Scheduler、SCM、PowerShell 或
  registry startup registration。
- Unsupported error wording now names startup registration specifically and points users to the implemented
  current user-session lifecycle commands: `service start/status/restart/stop`.
- This is diagnostics/error-boundary polish only; it does not implement install/uninstall or auto-start.

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
