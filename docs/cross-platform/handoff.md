# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近 commit

HEAD: `feat: add minimal gui backend shell`

## 当前 phase

Phase 9n: Minimal GUI Backend Shell 已实现并完成自动验证。下一步可以继续做 daemon
status snapshot command 的一个窄切片，或回到 Windows/Linux overlay PoC；不要直接做完整 GUI、
reconnect runtime、service management、配置编辑器或 release 打包指标。

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
- Phase 9f:
  - 更新 `docs/cross-platform/gui.md`，记录最小 library split 的范围、禁止方向和验收标准。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9f 状态。
  - 新增 `src/lib.rs`，只公开 `client_api`、`history`、`ipc`、`paths`、`state`、
    `text_stats`。
  - `src/ipc/mod.rs` 的 library surface 只公开 `client`、`protocol`、`transport`；`ipc::server`
    留在 binary 的内联 `ipc` 模块中。
  - `src/main.rs` 继续挂载 `ipc::server`，daemon runtime 可用路径不变。
  - `tests/platform_layout.rs` 增加最小 library surface 守卫，并把旧 9d 测试调整为继续禁止
    Tauri workspace / GUI runtime 依赖。
  - 未新增 IPC command/event，未 bump `PROTO_VERSION`，未新增 Tauri/WRY/WebView 依赖。
- Phase 9g:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI client 连接状态骨架范围：只描述 client
    side 状态、recoverable problem kind 和 retry delay，不实现后台 reconnect loop。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9g 状态。
  - `src/client_api.rs` 新增 `DaemonConnectionState`、`DaemonConnectionProblemKind`、
    `DaemonConnectionProblem`、`DEFAULT_RECONNECT_DELAYS_MS`、`next_reconnect_delay_ms()`、
    `reconnecting_state()` 和 daemon connection problem helper。
  - retry delay 是纯函数、短序列且有上限；`reconnecting_state()` 的 attempt 计数在极大输入下
    饱和到 `u32::MAX`。
  - `tests/platform_layout.rs` 增加 reconnect skeleton 架构守卫，确认 daemon/TUI 还未消费该
    GUI 状态骨架，且未引入 runtime loop 或 GUI runtime。
  - 未新增 IPC command/event，未 bump `PROTO_VERSION`，未创建 Tauri workspace，未改变 TUI
    连接行为。
- Phase 9h:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI backend event bridge 骨架范围：只把既有
    daemon `Event`、connection state 和 recoverable connection problem 封装成 GUI backend
    可转发事件。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9h 状态。
  - `src/client_api.rs` 新增 `GuiBackendEvent<'a>`，以及
    `gui_backend_event_from_daemon_event()`、`gui_backend_event_from_connection_state()`、
    `gui_backend_event_from_connection_problem()`。
  - daemon event bridge 复用 `classify_first_screen_event()`；bridge 只持有引用，不 clone 大型
    history payload，不生成 frontend view model，不调用 Tauri event API。
  - `tests/platform_layout.rs` 增加 bridge 架构守卫，确认未引入 Tauri/WRY/WebView、runtime loop
    或 protocol ownership。
  - 未新增 IPC command/event，未 bump `PROTO_VERSION`，未创建 Tauri workspace，未改变 TUI
    连接行为。
- Phase 9i:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI 首屏 metrics/timing 纯模型边界：时间戳由后续
    GUI backend 传入，shared client API 只做纯计算、饱和差值和首屏 readiness 判定。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9i 状态。
  - `src/client_api.rs` 新增 `FirstScreenReadiness`、`FirstScreenTimingMarks`、
    `FirstScreenTiming` 和纯 `from_marks()` helper。
  - 首屏 ready 的最小判定要求 daemon status、history page 和 history stats 都到达；snapshot、
    history changed 和 recoverable error 不会单独让首屏 ready。
  - helper 不调用系统时间、timer、IPC、Tauri event API 或 metrics sink；未新增 IPC
    command/event，未 bump `PROTO_VERSION`，未创建 Tauri workspace，未改变 TUI 连接行为。
- Phase 9j:
  - 基于 Tauri v2 文档更新 `docs/cross-platform/gui.md`，记录 capabilities/permissions
    preflight：capabilities 将 permissions 授权给指定 windows/webviews，permissions 显式开启
    frontend 可访问 command/plugin，并可包含 scopes。
  - 明确 GUI PoC 只给主 window/webview 绑定最小 capability，只暴露 shuohua GUI backend 自有
    command；frontend 不直接访问 IPC transport、history/config 文件或 daemon implementation。
  - 明确 PoC 不默认启用 shell、filesystem、http、process、global shortcut、updater、sidecar
    管理等宽权限；`core:default` 不作为默认授权策略，创建 workspace 时需先列出实际所需
    permission。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9j 状态。
  - `tests/platform_layout.rs` 增加文档/架构守卫，确认权限 preflight 已记录，且仍无
    `src-tauri/**` workspace 文件或 Tauri/WRY/WebView runtime 依赖。
- Phase 9k:
  - 基于 Tauri v2 build/bundle 文档更新 `docs/cross-platform/gui.md`，记录创建最小 Tauri
    workspace 前的验收清单。
  - 明确下一阶段只允许新增最小 GUI app 骨架、主 window/webview、最小 capabilities 文件和
    调用 shared `client_api` 的 backend shell；禁止同时实现完整页面、onboarding、配置编辑器、
    service management、sidecar、复制 IPC 类型或 daemon runtime 依赖。
  - 记录 release 指标清单：bundle path/type、unsigned/signed 状态、cold start、首屏 ready、
    open GUI idle RSS/CPU、关闭 GUI 后 daemon 存活、daemon 未打开 GUI 时无 WebView/Tauri
    进程。
  - `tests/platform_layout.rs` 增加文档/架构守卫，确认 workspace 前验收清单已记录，且仍无
    `src-tauri/**` workspace 文件或 Tauri/WRY/WebView runtime 依赖。
- Phase 9l:
  - 更新 `docs/cross-platform/gui.md`，记录后续 GUI backend 的 connection supervisor task
    ownership：首次连接 daemon、发送 `first_screen_commands()`、订阅 daemon event、应用
    `reconnecting_state()` 退避并通过 `GuiBackendEvent` 转发状态。
  - 明确 supervisor 属于 GUI 进程，不进入 daemon、TUI 或 shared `client_api`；取消 owner 是
    GUI window/app lifecycle，旧 task 的 late event 必须由 session id/generation 丢弃。
  - 明确 reconnect 只处理 recoverable client-side 问题：connect failed、event stream closed、
    read failed；不自动启动 daemon、不安装或重启 service、不修改配置。
  - 明确 timer、spawn、channel、Tauri event emission、metrics sink 只属于后续 GUI backend；
    shared `client_api` 继续只提供纯状态、退避、event bridge 和 timing helper。
  - `tests/platform_layout.rs` 增加文档/架构守卫，确认 reconnect ownership 已记录，且
    `src/client_api.rs` 仍无 runtime/GUI token。
- Phase 9m:
  - 更新 `docs/cross-platform/gui.md`，记录最小 Tauri workspace skeleton 的允许文件、权限边界
    和禁止项。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9m 状态。
  - 新增 `src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`src-tauri/build.rs`、
    `src-tauri/src/main.rs`、`src-tauri/src/lib.rs` 和 `src-tauri/capabilities/default.json`。
  - `src-tauri/Cargo.toml` 是独立 `shuohua-gui` crate，使用 Tauri v2，并通过
    `shuohua = { path = ".." }` 依赖根 crate；root `Cargo.toml` 未加入 workspace 或 Tauri
    dependency。
  - capabilities 只绑定主 window，权限保持在 `core:event:default`；未启用 shell、filesystem、
    http、process、global shortcut、updater 或 sidecar。
  - `tests/platform_layout.rs` 增加 Phase 9m skeleton 隔离测试，并把旧 Phase 9d 守卫调整为
    继续保护 root runtime 不引入 GUI runtime，而不是禁止 `src-tauri/**` 存在。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 frontend view model 或 reconnect supervisor。
- Phase 9n:
  - 更新 `docs/cross-platform/gui.md`，记录最小 GUI backend shell 和静态 frontend placeholder
    的边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9n 状态。
  - `src-tauri/src/lib.rs` 增加本地 `gui_shell_metadata` Tauri command，并通过
    `tauri::generate_handler!` 注册到 builder；command 只返回静态 metadata。
  - 新增 `gui-dist/index.html`，作为 `frontendDist` 的最小静态 placeholder；不引入 npm/vite、
    frontend dependency、dev server config 或完整页面。
  - `src-tauri/tauri.conf.json` 显式使用既有 `../assets/icon/shuohua-icon-1024.png`，并设置
    `bundle.active=false`，让 `cargo check --manifest-path src-tauri/Cargo.toml` 能通过 Tauri
    `generate_context!()` 的编译期 icon 检查，但仍不做 bundle。
  - 新增 `src-tauri/Cargo.lock`，锁定独立 GUI app crate 的 Tauri 依赖；`.gitignore` 忽略
    Tauri build script 生成的 `src-tauri/gen/` schema 目录。
  - `tests/platform_layout.rs` 增加 Phase 9n 架构守卫，确认 GUI shell 不连接 daemon、不拥有
    runtime loop，且 root/daemon/TUI/client_api 不引入 GUI runtime token。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 Status/History/Diagnostics view model 或 reconnect supervisor。

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
- Phase 9f 已跑：`cargo test --test platform_layout gui_minimal_library_split_exposes_only_client_protocol_surface`，
  先红灯失败于缺少 `src/lib.rs`，实现后通过。
- Phase 9f 已跑：`cargo test client_api::tests`，通过。该命令同时覆盖 `src/lib.rs` 和
  `src/main.rs` 中的 client API 单元测试。
- Phase 9f 已跑：`cargo test --test platform_layout`，通过 18 个测试。
- Phase 9f 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：89 个 library unit tests、636 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、18 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9g 已跑：`cargo test client_api::tests::daemon_connection_state_models_bounded_reconnect_without_protocol_changes`，
  先红灯失败于缺少 reconnect state 类型和 helper，实现后通过。
- Phase 9g 已跑：`cargo test --test platform_layout gui_reconnect_state_skeleton_lives_in_client_api_without_runtime_loop`，
  通过。
- Phase 9g 已跑：`cargo test client_api::tests`，通过 4 个 client API 单元测试。
- Phase 9g 已跑：`cargo test --test platform_layout`，通过 19 个测试。
- Phase 9g 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：90 个 library unit tests、637 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、19 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9h 已跑：`cargo test client_api::tests::gui_backend_event_bridge_wraps_existing_client_api_shapes`，
  先红灯失败于缺少 `GuiBackendEvent` 和 bridge helper，实现后通过。
- Phase 9h 已跑：`cargo test --test platform_layout gui_backend_event_bridge_lives_in_client_api_without_gui_runtime`，
  通过。
- Phase 9h 已跑：`cargo test client_api::tests`，通过 5 个 client API 单元测试。
- Phase 9h 已跑：`cargo test --test platform_layout`，通过 20 个测试。
- Phase 9h 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：91 个 library unit tests、638 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、20 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9i 已跑：`cargo test client_api::tests::first_screen_timing_models_readiness_without_runtime_or_protocol_changes`，
  先红灯失败于缺少 `FirstScreenReadiness`、`FirstScreenTimingMarks` 和 `FirstScreenTiming`，
  实现后通过。
- Phase 9i 已跑：`cargo test --test platform_layout gui_first_screen_metrics_timing_stays_pure_client_api`，
  先红灯失败于缺少 Phase 9i client API token，实现后通过。
- Phase 9i 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、21 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9j 已跑：`cargo test --test platform_layout gui_tauri_permissions_preflight_is_documented_without_workspace`，
  通过。
- Phase 9j 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、22 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9k 已跑：`cargo test --test platform_layout gui_tauri_workspace_pre_creation_acceptance_is_documented_without_workspace`，
  先红灯失败于缺少连续的进程边界 token，补文档后通过。
- Phase 9k 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、23 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9l 已跑：`cargo test --test platform_layout gui_reconnect_supervisor_ownership_is_documented_without_runtime_loop`，
  先红灯失败于缺少稳定 `connection supervisor` 和 `read failed` 文档 token，补文档后通过。
- Phase 9l 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、24 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9m 已跑：`cargo test --test platform_layout gui_minimal_tauri_workspace_skeleton_is_isolated_from_root_runtime`，
  先红灯失败于缺少 `src-tauri/Cargo.toml`，实现后通过。
- Phase 9m 已跑：`rg -n "tauri|wry|webview|WebView|tao" Cargo.toml src/daemon src/tui src/client_api.rs`，
  无命中。
- Phase 9m 已跑：`cargo test --test platform_layout`，通过 25 个测试。
- Phase 9n 已跑：`cargo test --test platform_layout gui_backend_shell_placeholder_stays_local_to_tauri_app`，
  先红灯失败于缺少 `#[tauri::command]`，实现后通过。
- Phase 9n 已跑：`cargo test --test platform_layout gui_minimal_tauri_workspace_skeleton_is_isolated_from_root_runtime`，
  通过。
- Phase 9n 已跑：`cargo test --test platform_layout`，通过 26 个测试。
- Phase 9n 已跑：`cargo check --manifest-path src-tauri/Cargo.toml`。第一次失败于
  `generate_context!()` 找不到默认 `src-tauri/icons/icon.png`；改为显式使用已有
  `assets/icon/shuohua-icon-1024.png` 后通过。
- Phase 9n 已跑：`rg -n "tauri|wry|webview|WebView|tao" Cargo.toml src/daemon src/tui src/client_api.rs`，
  无命中。
- Phase 9n 已跑：`rg -n "connect_default|DaemonClient|ipc::client|Command::|Event::|tokio::spawn|tokio::time|std::thread::spawn" src-tauri`，
  无命中。
- Phase 9n 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、26 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
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
- Phase 9f 已创建最小 library target，但 surface 仍包含现有 `history` / `state` 模型，而不是
  更小 wire DTO；这避免协议复制，但也意味着 GUI backend 会看到这些数据模型。
- Phase 9g 只提供连接状态/退避骨架，没有实现真实后台 reconnect task、Tauri event bridge
  或 daemon offline UI view model。
- Phase 9h 只提供 GUI backend event bridge 的纯封装，没有实现 Tauri event emission、
  frontend view model 或后台 reconnect loop。
- Phase 9i 只提供首屏 metrics/timing 纯模型，没有实现真实 metrics sink、Tauri event
  emission、前端展示、后台 reconnect loop 或打包指标采集。
- Phase 9j 只记录 Tauri permissions/capabilities preflight，没有创建真实 Tauri workspace、
  capabilities JSON、frontend command binding 或打包验证。
- Phase 9k 只记录创建 Tauri workspace 前的验收清单，没有创建真实 Tauri workspace、
  capabilities JSON、frontend command binding、release build 或打包验证。
- Phase 9l 只记录 reconnect supervisor ownership/cancellation 语义，没有实现真实 runtime loop、
  Tauri event emission、frontend view model 或 metrics sink。
- Phase 9m/9n 只创建最小 `src-tauri/**` skeleton、静态 placeholder 和本地 metadata command；
  尚未运行 `tauri dev` / `tauri build` / `tauri bundle`，也没有启动 GUI 或 daemon。后续需要
  单独决定何时运行 release build、如何记录 cold start/RSS/CPU/bundle 指标。
- Phase 9n 的 `gui_shell_metadata` 只验证本地 command wiring，不连接 daemon、不读
  config/history、不生成真实 Status/History/Diagnostics view model。
- `ipc::transport` 仍是 Unix-only，library client 只实际覆盖 macOS/Linux 当前 transport。
  Windows Named Pipe adapter 仍是后续 IPC transport backend 工作。
- `current_platform_capabilities()` 是 Phase 1 静态快照，不执行权限 probe；后续消费方不要把
  静态 `desktop.permissions=available` 误解为当前已授权。
- `overlay::renderer::renderer_capabilities()` 同样是静态快照，不创建窗口、不 probe 当前
  compositor/权限、不读取业务配置。

## 下一步

Phase 9n 后，进入下一小步：

- 若继续 GUI，下一阶段只能做 daemon status snapshot command 或 first-screen command wiring
  的一个窄切片；继续禁止 daemon 热路径引入 WebView，且不要启动 daemon、GUI 或 release 打包。
- 若目标平台环境可用，也可以先按 Phase 7a/8a checklist 做 Windows/Linux 最小 overlay PoC。

建议下一 session prompt：

```text
继续 /Users/ghot/repo/shuohua 跨平台改造，当前在 feat/cross-platform-design。
先读 AGENTS.md、TODO、docs/cross-platform/README.md、overview.md、
development-plan.md、overlay.md、platform-capabilities.md、macos-baseline.md、
handoff.md。
Phase 9n Minimal GUI Backend Shell 已实现；先查看最新 commit 和验证结果。
下一步在 daemon status snapshot command / first-screen command wiring / Windows/Linux overlay PoC 之间做一个小步计划。
```
