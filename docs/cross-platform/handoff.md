# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近 commit

HEAD: `docs: record gui library boundary preconditions`

## 当前 phase

Phase 9e: GUI Library Split Audit Baseline 已实现，提交前验证中。下一步应先做 library
split 最小实现，或继续停留在文档化 PoC，不要直接创建 Tauri workspace。

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
  - 新增 `src/ipc/transport.rs`，集中 macOS/Linux 当前 UDS endpoint、connect、bind、accept
    和 stale endpoint 清理。
  - `src/ipc/client.rs` / `src/ipc/server.rs` 不再直接 import `tokio::net::UnixStream` /
    `UnixListener`，JSON-line protocol 未改变。
- Phase 4a:
  - 更新 `docs/cross-platform/ipc-service.md`，把 Phase 4 拆成 lock/process probe facade 和
    后续 service manager facade。
  - 新增 `src/platform/lifecycle.rs`，集中 daemon lock file + `flock` 和 process probe
    `kill(pid, 0)` 语义。
  - 删除旧 `src/daemon/lock.rs`，`daemon::process` 改用 `platform::lifecycle::acquire_daemon_lock()`。
  - `cli::service::macos` 的 wait-for-exit 改用 `platform::lifecycle::process_exists()`，
    macOS stop/restart/status 用户可见语义不变。
  - `tests/platform_layout.rs` 增加 daemon lifecycle primitive import 边界测试。
- Phase 4b:
  - 更新 `docs/cross-platform/ipc-service.md`，记录 `platform::service` facade 边界。
  - 新增 `src/platform/service.rs`，集中 service manager backend 选择；macOS backend 继续使用
    launchd user agent。
  - `src/cli/service/mod.rs` 保留 clap command、命令分发和 `launchd_status()` 兼容入口，不再
    拥有 launchd 或 unsupported backend 文件。
  - 删除旧 `src/cli/service/macos.rs` / `src/cli/service/unsupported.rs`。
  - `tests/platform_layout.rs` 增加 service manager import 边界测试。
- Phase 5a:
  - 更新 `docs/cross-platform/platform-capabilities.md`，把 Phase 5 拆成 5a desktop facade 和
    5b hotkey provider facade。
  - 新增 `src/platform/desktop.rs`，聚合 active app、clipboard、text injection 和 permission
    primitives。
  - `voice::dispatch`、`voice::engine`、`platform::daemon`、`tui::history` 和 `cli::doctor`
    改用 `platform::desktop`。
  - 删除 `src/post/app_context.rs`；`post::AppContext` 保留为 post pipeline 数据模型，
    前台 App 查询归 desktop capability。
  - `tests/platform_layout.rs` 增加 desktop facade import 边界测试。
- Phase 5b:
  - 更新 `docs/cross-platform/platform-capabilities.md`，记录 hotkey provider facade 的边界。
  - 新增 `src/platform/hotkey.rs`，集中 hotkey provider backend 选择、OS thread spawn 和
    非 macOS unsupported fallback。
  - `src/platform/daemon.rs` 不再直接知道 `provider_darwin`、thread 名称或 unsupported 文案。
  - macOS 仍调用 `hotkey::provider_darwin::run()`；CGEventTap callback、pipe wire format、
    `Suppressor` 和 `TrackerSet` 未改变。
  - `tests/platform_layout.rs` 增加 hotkey provider facade import 边界测试。
- Phase 6a:
  - 更新 `docs/cross-platform/overlay.md` 和 `docs/modules/overlay.md`，记录 renderer facade
    边界。
  - 新增 `src/overlay/renderer.rs`，集中 overlay renderer backend 选择和非 macOS
    unsupported fallback。
  - `src/overlay/mod.rs` 的 `run()` 保持上层 API 不变，只转发到 `overlay::renderer`。
  - macOS backend 仍调用 `overlay::macos::run()`；AppKit view/chrome/icon_fx、动画、
    窗口层级、focused window 锚定和 material fallback 未改变。
  - `tests/platform_layout.rs` 增加 overlay renderer facade import 边界测试。
- Phase 6b:
  - 更新 `docs/cross-platform/overlay.md`、`docs/modules/overlay.md`、
    `docs/cross-platform/platform-capabilities.md` 和 `docs/cross-platform/overview.md`，
    记录 renderer capability skeleton 边界。
  - `src/overlay/renderer.rs` 新增只读 `renderer_capabilities()` 静态快照，复用
    `platform::capability` 的 `CapabilityStatus` / `CapabilityId` / status kind。
  - 新增 `MaterialPreference` 和 `MATERIAL_FALLBACK_ORDER`，固定
    `liquid_glass -> blurred_glass -> translucent -> solid` 的建模顺序。
  - macOS snapshot 描述当前 AppKit backend；非 macOS 仍是 structured unsupported。
  - macOS `overlay::run()` 仍调用 `overlay::macos::run()`；未修改 AppKit renderer、
    `OverlayCmd`、`OverlayModel`、layout 或 theme parser。
  - `tests/platform_layout.rs` 增加 renderer capability skeleton 边界测试。
- Phase 6c:
  - 更新 `docs/cross-platform/overlay.md`、`docs/cross-platform/platform-capabilities.md` 和
    `docs/cross-platform/overview.md`，记录 doctor 只读消费 renderer capability snapshot。
  - `src/overlay/mod.rs` 对 crate 内暴露 `renderer_capabilities()`。
  - `src/cli/doctor.rs` 的 capability summary 先读全局静态快照，再用 renderer snapshot
    覆盖同 `CapabilityId` 的 overlay 条目。
  - doctor 错误/警告计数、退出码、IPC/daemon/overlay 运行路径不变；TUI/GUI 未接入。
  - `tests/platform_layout.rs` 增加 renderer capability 仅由 doctor 消费的边界测试。
- Phase 7a:
  - 更新 `docs/cross-platform/overlay.md`，基于 Microsoft 文档记录 Windows overlay PoC
    baseline：Win32 popup/top-level window、extended styles、layered alpha、SetWindowPos
    topmost、WM_NCHITTEST click-through、Mica/DWM backdrop 降级判断和 capture exclusion。
  - 更新 `docs/cross-platform/development-plan.md`，把 Phase 7 拆出 7a 文档化 baseline。
  - 更新 `docs/cross-platform/overview.md`，记录 Phase 7a 当前状态。
  - 未新增 Windows renderer 文件，未引入依赖，未修改 macOS overlay 或 daemon 热路径。
- Phase 8a:
  - 更新 `docs/cross-platform/overlay.md`，基于 Wayland core/xdg-shell、wlr layer-shell、
    GTK Layer Shell、KDE LayerShellQt/KDE plasma shell protocol 和 GNOME Mutter issue
    记录 Linux Wayland overlay PoC baseline。
  - 记录 wlroots/KDE/GNOME/X11 的验证 checklist，并明确 GNOME Wayland 和普通 xdg-shell
    不应假设支持任意置顶 overlay。
  - 更新 `docs/cross-platform/development-plan.md`，把 Phase 8 拆出 8a 文档化 baseline。
  - 更新 `docs/cross-platform/overview.md`，记录 Phase 8a 当前状态。
  - 未新增 Linux renderer 文件，未引入 Wayland crate，未修改 macOS overlay 或 daemon 热路径。
- Phase 9a:
  - 更新 `docs/cross-platform/gui.md`，基于 Tauri v2 文档记录 GUI PoC baseline：
    独立按需 client、command/event 桥接、permissions/capabilities、sidecar 非默认路线、
    release build/bundle 指标和 TUI 回退。
  - 更新 `docs/cross-platform/development-plan.md`，把 Phase 9 拆出 9a 文档化 baseline。
  - 更新 `docs/cross-platform/overview.md`，记录 Phase 9a 当前状态。
  - 未新增 Tauri workspace，未引入 WebView runtime，未修改 daemon/CLI/TUI。
- Phase 9b:
  - 更新 `docs/cross-platform/gui.md`，记录共享 daemon client API 边界：只封装现有
    `ipc::protocol::Command` / `Event`，不新增 wire shape，不 bump `PROTO_VERSION`。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，
    记录 Phase 9b 的范围和状态。
  - 新增 `src/client_api.rs`，作为 TUI 和后续 GUI backend 复用的 daemon client 入口。
  - `src/tui/mod.rs` 改为通过 `client_api::DaemonClient` 获取 client 类型，startup command
    通过 `client_api::subscribe_command()` 构造；TUI 行为和 IPC protocol 不变。
  - `tests/platform_layout.rs` 增加 GUI client API 边界测试，禁止 daemon/TUI/shared client
    path 引入 Tauri、WRY、WebView 或 `tao` token，并确认 `Cargo.toml` 未新增相关依赖。
- Phase 9c:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI 首屏 helper 边界：request helper 只返回
    现有 `Command`，response classifier 只分类现有 `Event`，不做本地化、不读取
    config/history 文件、不生成 frontend view model。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，
    记录 Phase 9c 的范围和状态。
  - `src/client_api.rs` 增加 `first_screen_commands(history_limit)`，映射到
    `Subscribe`、`DaemonStatus`、`GetHistory` 和 `GetHistoryStats`。
  - `src/client_api.rs` 增加 `FirstScreenEvent` 和 `classify_first_screen_event()`，把
    `Snapshot`、`DaemonStatus`、`History`、`HistoryStats`、`HistoryChanged` 和 `Error`
    分类为 GUI backend 可消费的首屏输入。
  - `src/main.rs` 将 `client_api` 公开为 crate 边界，供后续 GUI backend 复用；未新增
    Tauri workspace 或 GUI runtime 依赖。
  - `tests/platform_layout.rs` 增加首屏 helper 架构测试，确认 helper 仍位于 `client_api`，
    不拥有 protocol version，也不引入 Tauri/WRY/WebView/`tao` token。
- Phase 9d:
  - 更新 `docs/cross-platform/gui.md`，明确当前 crate 只有 binary target，`client_api`
    仍是 binary crate 内边界，不是外部 Tauri crate 可依赖的 library API。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，
    记录真正创建 Tauri workspace 前必须先做 library split 评审。
  - 记录 library split 的最小候选 surface：`client_api`、`ipc::client`、`ipc::protocol`、
    `ipc::transport` 和必要数据模型；禁止把 daemon runtime、hotkey、overlay、voice、
    AppKit 或 TUI 拉进 GUI backend 依赖树。
  - `tests/platform_layout.rs` 增加当前边界保护测试：没有 `src/lib.rs`、没有 Tauri workspace
    文件、`Cargo.toml` 仍只有既有 `shuo` binary target 且不含 GUI runtime 依赖。
- Phase 9e:
  - 更新 `docs/cross-platform/gui.md`，记录 library split audit baseline。
  - 记录最小候选 library surface：`client_api`、`ipc::client`、`ipc::protocol`、
    `ipc::transport` 和必要数据模型，足够后续 GUI backend 连接 daemon、发送首屏命令、
    接收并分类首屏事件。
  - 记录阻塞点：`ipc::protocol` 依赖 `history` / `state` 模型，不能只移动 protocol 文件；
    `ipc::transport` 当前是 Unix-only transport，Windows Named Pipe backend 仍属后续 IPC
    transport backend 阶段。
  - 继续禁止在 library split 前创建 Tauri workspace，避免复制 IPC 类型或绕过 `client_api`。
  - `tests/platform_layout.rs` 增加 audit 文档守卫，确认 GUI 文档记录最小 surface、阻塞点和
    禁止方向。

## 验证结果

- 已跑：`cargo test --test platform_layout daemon_lifecycle_primitives_live_behind_platform_facade`，通过。
- 已跑：`cargo test --test platform_layout service_manager_lives_behind_platform_facade`，通过。
- 已跑：`cargo test platform::service::`，通过 12 个测试。
- 已跑：`cargo test cli::service::`，通过 1 个测试。
- 已跑：`cargo test platform::lifecycle`，通过 2 个测试。
- Phase 4a 曾跑：`cargo test cli::service::macos::tests`，通过 12 个测试；Phase 4b 后这些
  测试已随实现迁移到 `platform::service::`。
- 已跑：`cargo test --test platform_layout desktop_capabilities_live_behind_platform_desktop_facade`，
  先红灯失败于缺少 `src/platform/desktop.rs`，实现后通过。
- 已跑：`cargo test --test platform_layout hotkey_provider_lives_behind_platform_hotkey_facade`，
  先红灯失败于缺少 `src/platform/hotkey.rs`，实现后通过。
- 已跑：`cargo test --test platform_layout overlay_renderer_lives_behind_renderer_facade`，
  先红灯失败于缺少 `src/overlay/renderer.rs`，实现后通过。
- 已跑：`cargo test --test platform_layout overlay_renderer_capabilities_live_with_renderer_facade`，
  先红灯失败于缺少 `renderer_capabilities`，实现后通过。
- 已跑：`cargo test overlay::renderer`，通过 3 个 renderer 单元测试。
- 已跑：`cargo test cli::doctor::tests`，通过 7 个测试。
- 已跑：`cargo test hotkey`，通过 81 个测试。
- 已跑：`cargo test overlay`，通过 45 个 unit tests，另外 integration tests 过滤项正常。
- 已跑：`cargo test --test doc_consistency`，通过 2 个测试。
- 已跑：`cargo test --test platform_layout`，通过 13 个测试。
- 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，通过。
  `cargo test` 覆盖：633 个 unit tests、5 个 `apple_helper_build` tests、
  1 个 `cli_runtime_boundary` test、2 个 `doc_consistency` tests、13 个 `platform_layout` tests、
  6 个 `theme_registry_build` tests。
- Phase 9b 已跑：`cargo test --test platform_layout gui_client_api_boundary_stays_out_of_daemon_hot_path`，
  先红灯失败于缺少 `src/client_api.rs`，实现后通过。
- Phase 9b 已跑：`cargo test client_api::tests`，通过 1 个 client API 单元测试。
- Phase 9b 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 9c 已跑：`cargo test client_api::tests`，先红灯失败于缺少
  `first_screen_commands`、`classify_first_screen_event` 和 `FirstScreenEvent`，实现后通过
  3 个 client API 单元测试。
- Phase 9c 已跑：`cargo test --test platform_layout gui_first_screen_helpers_live_in_client_api_without_gui_runtime`，
  通过。
- Phase 9c 已跑：`cargo test --test platform_layout`，通过 15 个测试。
- Phase 9c 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 9d 已跑：`cargo test --test platform_layout gui_library_boundary_is_not_split_before_design_review`，
  通过。
- Phase 9d 已跑：`cargo test --test platform_layout`，通过 16 个测试。
- Phase 9e 已跑：`cargo test --test platform_layout gui_library_split_audit_records_minimal_surface_and_blockers`，
  先红灯失败于缺少 Phase 9e 文档，补文档后通过。
- Phase 9e 已跑：`cargo test --test platform_layout`，通过 17 个测试。
- macOS 权限、录音、overlay、clipboard/paste、TUI、service lifecycle、history 手动体验：未执行，
  需用户在真实 macOS 会话按 `macos-baseline.md` checklist 验证。

## 已知风险

- `src/daemon/fallback.rs` 仍用 `std::os::unix::net::UnixStream` 做 smart fallback endpoint probe；
  这是后续 smart fallback lifecycle 目标，不在 Phase 4 抽。
- `src/cli/doctor.rs` 仍有 launchd-centric 诊断输出；service manager facade 后应通过
  capability/status 和 service manager 模型收敛。
- Phase 5b 只抽 hotkey provider 启动边界，没有实现 Linux/Windows global hotkey backend。
- Phase 6c 只把 renderer capability snapshot 接入 doctor summary，没有实现 Windows/Linux
  overlay renderer 骨架，也没有接入 TUI/GUI。
- Phase 7a 只是 Microsoft 文档基线，不代表已在 Windows 11/10 真机验证。实际 topmost、
  no-activate、click-through、材质、capture exclusion 和性能数据仍需 PoC 记录。
- Phase 8a 只是 Wayland/layer-shell 文档基线，不代表已在 wlroots/KDE/GNOME 真机验证。
  实际 layer-shell availability、top layer、pointer passthrough、alpha、screen anchor 和性能
  数据仍需 PoC 记录。
- Phase 9a 只是 Tauri v2 文档基线，不代表已测 GUI 冷启动、内存、CPU、包体或三端打包。
  GUI PoC 仍需证明 daemon 未打开 GUI 时不加载 WebView，且 GUI 退出不影响 daemon。
- Phase 9c 只提供首屏 command helper 和 event classifier；尚未实现真实 Tauri GUI app、
  frontend view model、重连策略、指标采集或打包验证。
- Phase 9d 明确当前还没有 library target；独立 Tauri backend 不能直接依赖当前 binary
  crate 内的 `client_api`。下一步若要做 GUI app，先做 library split，而不是复制 IPC 类型。
- Phase 9e 仍未创建 library target。最小 split 会被 `history` / `state` 模型依赖和 Unix-only
  transport 约束；需要先决定是暴露这些数据模型，还是拆出更小 wire DTO。
- `current_platform_capabilities()` 是 Phase 1 静态快照，不执行权限 probe；后续消费方不要把
  静态 `desktop.permissions=available` 误解为当前已授权。
- `overlay::renderer::renderer_capabilities()` 同样是静态快照，不创建窗口、不 probe 当前
  compositor/权限、不读取业务配置。

## 下一步

提交 Phase 9e 后，进入下一小步：

- 若继续 GUI 路线，先做 library split 最小实现：创建 library target，只暴露经审计的
  client/protocol surface，并保护 daemon/TUI/GUI 依赖方向。
- 若暂不拆 library，可以停在文档化 GUI PoC，避免创建只能复制 IPC 类型的 Tauri workspace。
- 若继续 shared client API，先设计可恢复连接错误和重连状态，不新增 daemon protocol。
- 若目标平台环境可用，也可以先按 Phase 7a/8a checklist 做 Windows/Linux 最小 overlay PoC。

建议下一 session prompt：

```text
继续 /Users/ghot/repo/shuohua 跨平台改造，当前在 feat/cross-platform-design。
先读 AGENTS.md、TODO、docs/cross-platform/README.md、overview.md、
development-plan.md、overlay.md、platform-capabilities.md、macos-baseline.md、
handoff.md。
Phase 9e GUI Library Split Audit Baseline 已实现；先查看最新 commit 和验证结果。
下一步在 library split 最小实现、client_api 重连状态设计、或 Windows/Linux overlay PoC
之间做一个小步计划。
```
