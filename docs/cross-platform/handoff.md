# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近 commit

HEAD: `feat: add ipc transport facade`

## 当前 phase

Phase 3: IPC Transport Boundary 已完成并提交。下一步进入 Phase 4。

## 已完成事项

- Phase 0:
  - 新增 `docs/cross-platform/macos-baseline.md`，记录自动验证基线、macOS 手动验证 checklist、
    当前允许的 macOS-only 边界和后续阶段要处理的遗留边界。
  - 在 `docs/cross-platform/README.md` 增加 macOS baseline 阅读路由。
  - 扩展 `tests/platform_layout.rs`，保护 shared platform facade 和 macOS-only import 边界。
- Phase 1:
  - 新增 `src/platform/capability.rs`，提供共享 capability/status 类型和静态快照。
  - macOS 快照映射现有 backend；非 macOS 快照返回 `unsupported` +
    `backend_not_implemented`。
  - `shuo doctor` 只读打印 capability summary，不改变错误/警告计数或控制流。
- Phase 2:
  - 稳定 config/theme 跨平台规则，starter config 不默认输出 `[dev]`。
  - theme schema 增加受控的 `overlay.windows.material` / `overlay.linux.material` future 平台字段。
- Phase 3:
  - 更新 `docs/cross-platform/ipc-service.md`，明确 transport facade 边界、stale endpoint
    cleanup 暂时归属和非目标。
  - 新增 `src/ipc/transport.rs`，集中 macOS/Linux 当前 UDS endpoint、connect、bind、accept
    和 stale endpoint 清理。
  - `src/ipc/client.rs` / `src/ipc/server.rs` 不再直接 import `tokio::net::UnixStream` /
    `UnixListener`，只处理 JSON-line command/event 读写和 server dispatch。
  - `src/daemon/runtime.rs`、TUI、doctor、macOS service status/stop 改用 transport endpoint /
    `IpcClient::connect_default()`。
  - `tests/platform_layout.rs` 增加 IPC transport import 边界测试。

## 验证结果

- 已跑：`cargo test --test platform_layout`，通过 6 个测试。
- 已跑：`cargo test ipc::transport`，通过 3 个 transport 测试。
- 已跑：`cargo test ipc::server::tests`，通过 17 个 server 测试。
- 已跑：`cargo test daemon::fallback::tests`，通过 1 个 fallback 测试。
- 已跑：`cargo fmt --check`，通过。
- 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- 已跑：`cargo test`，通过：629 个 unit tests、5 个 `apple_helper_build` tests、
  1 个 `cli_runtime_boundary` test、2 个 `doc_consistency` tests、6 个 `platform_layout` tests、
  6 个 `theme_registry_build` tests。
- macOS 权限、录音、overlay、clipboard/paste、TUI、service lifecycle、history 手动体验：未执行，
  需用户在真实 macOS 会话按 `macos-baseline.md` checklist 验证。

## 已知风险

- `src/daemon/fallback.rs` 仍用 `std::os::unix::net::UnixStream` 做 smart fallback endpoint probe；
  这是 Phase 4 process probe / lifecycle 目标，不在 Phase 3 提前抽。
- `src/daemon/lock.rs` 仍是 lock file 实现；Phase 4 需要与 stale endpoint cleanup 顺序一起评审。
- `src/cli/doctor.rs` 仍有 launchd-centric 诊断输出；Phase 4 后应通过 capability/status
  和 service manager 模型收敛。
- `src/post/app_context.rs` 当前作为 post 平台入口直接转发到 macOS app context；Phase 5
  desktop capability boundary 可统一 facade。
- `current_platform_capabilities()` 是 Phase 1 静态快照，不执行权限 probe；后续消费方不要把
  静态 `desktop.permissions=available` 误解为当前已授权。

## 下一步

进入 Phase 4: Single Instance, Process Probe, Service Manager。

建议下一 session prompt：

```text
继续 /Users/ghot/repo/shuohua 跨平台改造，当前在 feat/cross-platform-design。
先读 AGENTS.md、TODO、docs/cross-platform/README.md、overview.md、
development-plan.md、ipc-service.md、platform-capabilities.md、macos-baseline.md、
handoff.md。
从 Phase 4 Single Instance, Process Probe, Service Manager 开始：先更新 ipc-service.md，
再写最小测试，最后抽 daemon lock/process probe/service manager 边界。不要改变
`shuo app service` 用户可见语义，不要改 IPC JSON-line protocol，不要自动安装
Linux/Windows service。
```
