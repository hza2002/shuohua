# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近 commit

HEAD: `feat: add windows named pipe transport`

## 当前 phase

GUI PoC 冻结，当前主线回到非 macOS 可用性。
Phase 3c Windows Named Pipe transport compile backend 已完成一个最小阶段：Windows target
使用 Tokio Named Pipe transport 编译通过，但 runtime/ACL/smart fallback 仍需 Windows 实机或 VM 验证。

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
- Phase 3c:
  - 更新 `docs/cross-platform/ipc-service.md`，记录 Windows Named Pipe transport compile backend
    的范围和未验证项。
  - Windows `ipc::transport` 从 placeholder `DuplexStream` 改为 Tokio
    `tokio::net::windows::named_pipe`。
  - server `accept()` 在当前 pipe instance 连接后创建下一条 pipe instance，再把已连接 stream
    交给既有 IPC server；client `connect()` 遇到 pipe busy 做短退避重试。
  - 该阶段不实现 Named Pipe ACL/security descriptor、不实现 Windows daemon single instance、
    不实现 smart fallback service 启动，也不声明 Windows runtime 可用。
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
- Phase 9o:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI first-screen request plan command 的边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9o 状态。
  - `src-tauri/src/lib.rs` 增加 `gui_first_screen_request_plan` Tauri command，复用
    `shuohua::client_api::first_screen_commands()` 生成首屏请求计划 summary。
  - request plan 只返回 command kind、history limit、requires daemon connection 和 transport
    opened=false；不创建 `DaemonClient`，不调用 `connect_default()`，不发送 IPC，不订阅 event
    stream。
  - `gui-dist/index.html` 展示 request plan command count/kinds 和静态连接字段；仍不实现真实
    Status/History/Diagnostics view model。
  - `tests/platform_layout.rs` 增加 Phase 9o 架构守卫，并调整 9n 守卫以允许 9o 在 `src-tauri`
    内对既有 `Command` 做 summary 映射。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 reconnect supervisor。
- Phase 9p:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI daemon status snapshot shape command 的边界：
    这是 shape preflight，不是真实 status client。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9p 范围和状态。
  - `src-tauri/src/lib.rs` 增加 `gui_daemon_status_snapshot` Tauri command，返回静态
    `connected=false`、`transport_opened=false`、`snapshot_available=false`、
    `state_label=disconnected`，并标记后续真实请求使用既有 `Command::DaemonStatus`。
  - `gui-dist/index.html` 展示 status snapshot shape；仍不实现真实 Status/History/Diagnostics
    view model。
  - `tests/platform_layout.rs` 增加 Phase 9p 架构守卫，确认 command 不创建 `DaemonClient`、
    不调用 `connect_default()`、不发送 IPC、不订阅 event stream、不启动 spawn/timer。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 reconnect supervisor 或 service management。
- Phase 9q:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI daemon status event mapper 的边界：只把调用方
    已拿到的既有 `Event::DaemonStatus` 映射成 Phase 9p 的 status snapshot response shape。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9q 范围和状态。
  - `src-tauri/src/lib.rs` 新增纯 `gui_daemon_status_snapshot_from_event()` mapper 和
    `wire_state_label()` helper；mapper 只处理 `Event::DaemonStatus`，其他 event 返回 `None`。
  - `GuiDaemonStatusSnapshot` 增加 `pid`、`uptime_ms`、`recording_id` 可选字段；9p 的
    `gui_daemon_status_snapshot` 继续通过 empty helper 返回未连接静态 shape。
  - 新增 Tauri crate 单元测试覆盖 `Event::DaemonStatus` 到 snapshot shape 的映射，以及
    `HistoryChanged` 不被误处理。
  - `tests/platform_layout.rs` 增加 Phase 9q 架构守卫，确认 mapper 不创建 `DaemonClient`、
    不调用 `connect_default()`、不发送 IPC、不订阅 event stream、不启动 spawn/timer。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现真实 status request、reconnect supervisor 或 service management。
- Phase 9r:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI daemon status one-shot request command 边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9r 范围和状态。
  - `src-tauri/src/lib.rs` 增加 `gui_daemon_status_request_once` Tauri command：显式调用时通过
    `DaemonClient::connect_default()` 连接现有 daemon IPC，发送既有 `Command::DaemonStatus`，
    用 `recv_until` 等待 `Event::DaemonStatus` 并复用 9q mapper 返回 snapshot shape。
  - 新增 `GuiDaemonStatusRequestError` recoverable error shape，覆盖 connect/write/read failure、
    daemon `Event::Error` 和 daemon closed。
  - placeholder `gui-dist/index.html` 不自动调用 one-shot command，避免打开静态页面时默认连接
    daemon。
  - `tests/platform_layout.rs` 增加 Phase 9r 架构守卫，确认 one-shot command 显式存在但不发送
    `Subscribe`、不订阅 event stream、不启动 spawn/timer/reconnect loop。
  - Tauri crate 单元测试覆盖 status event mapping 和 recoverable error shape。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 frontend Status view model、reconnect supervisor 或 service
    management。
- Phase 9s:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI history summary one-shot request command 边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9s 范围和状态。
  - `src-tauri/src/lib.rs` 增加 `gui_history_summary_request_once` Tauri command：显式调用时通过
    `DaemonClient::connect_default()` 连接现有 daemon IPC，发送既有
    `Command::GetHistory { limit, before: None, before_id: None, query: None }` 和
    `Command::GetHistoryStats`，用 `recv_until` 等待 `Event::History` / `Event::HistoryStats`
    并返回最小 history summary shape。
  - 新增 `GuiHistorySummaryRequestError` recoverable error shape，覆盖 connect/write/read failure、
    daemon `Event::Error` 和 daemon closed。
  - summary 只包含 page count、matched、aggregate stats、latest record id/status/text preview
    和 request metadata；不实现搜索、分页 cursor、详情、audio 管理、图表或本地化。
  - placeholder `gui-dist/index.html` 不自动调用 one-shot command，避免打开静态页面时默认连接
    daemon 或读取 history。
  - `tests/platform_layout.rs` 增加 Phase 9s 架构守卫，确认 one-shot command 显式存在但不发送
    `Subscribe`、不订阅 event stream、不启动 spawn/timer/reconnect loop。
  - Tauri crate 单元测试覆盖 history summary event mapping 和 recoverable error shape。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 frontend History view model、reconnect supervisor 或 service
    management。
- Phase 9t:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI first-screen summary one-shot request command
    边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9t 范围和状态。
  - `src-tauri/src/lib.rs` 增加 `gui_first_screen_summary_request_once` Tauri command：显式调用时
    通过一次 `DaemonClient::connect_default()` 连接现有 daemon IPC，发送既有
    `Command::DaemonStatus`、`Command::GetHistory { limit, before: None, before_id: None,
    query: None }` 和 `Command::GetHistoryStats`，用 `recv_until` 等待 `Event::DaemonStatus` /
    `Event::History` / `Event::HistoryStats` 并返回组合 first-screen summary shape。
  - summary 复用 9r status snapshot shape 和 9s history summary shape，并带 history limit、
    availability 和 request metadata；不实现 loading/retry UI、metrics 展示、event stream、
    搜索、详情、audio 管理或本地化。
  - 新增 `GuiFirstScreenSummaryRequestError` recoverable error shape，覆盖 connect/write/read
    failure、daemon `Event::Error` 和 daemon closed。
  - placeholder `gui-dist/index.html` 不自动调用 one-shot command，避免打开静态页面时默认连接
    daemon 或读取 history。
  - `tests/platform_layout.rs` 增加 Phase 9t 架构守卫，确认 one-shot command 显式存在但不发送
    `Subscribe`、不订阅 event stream、不启动 spawn/timer/reconnect loop。
  - Tauri crate 单元测试覆盖 first-screen summary event mapping 和 recoverable error shape。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 frontend Status/History view model、reconnect supervisor 或 service
    management。
- Phase 9u:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI first-screen summary request timing 的边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9u 范围和状态。
  - `src-tauri/src/lib.rs` 的 `GuiFirstScreenSummary` 增加 `timing` 字段，类型为
    `GuiFirstScreenSummaryTiming`，包含 `connectDurationMs`、`firstEventMs`、`readyMs` 和
    `requestDurationMs`。
  - `gui_first_screen_summary_request_once` 在本次显式 command invocation 内使用
    `std::time::Instant` 记录 request start、connect completed、first matched daemon event 和
    summary ready 的 elapsed milliseconds。
  - timing 只附着在 9t 的 first-screen summary response 上；不进入 daemon protocol、
    shared `client_api`、history、trace 或 metrics sink。
  - 未使用 `tokio::time`，未启动 timer task，未订阅 event stream，未实现 reconnect loop、
    loading/retry UI 或 frontend view model。
- Phase 10a:
  - `Makefile` 新增 `make check-windows` 和 `make check-linux`，作为跨平台 cfg/type 边界检查入口。
  - shared network clients 改为 target-specific TLS：Linux 使用 Rustls，非 Linux 保持 native TLS。
  - `shuo doctor` 的 platform capability summary 增加 unsupported/unavailable detail 行，包含
    backend、reason 和可选 next step，方便 skeleton 阶段诊断。
  - `tests/platform_layout.rs` 增加 network TLS 配置守护测试，避免 Linux check 路径重新引入
    OpenSSL-backed native TLS。
- Phase 10b:
  - 更新 `docs/cross-platform/development-plan.md`，记录 TUI capability diagnostics 的只读边界。
  - TUI Status 页新增 `Platform` 区块，合并 `current_platform_capabilities()` 和
    `overlay::renderer_capabilities()` 后显示 available/unsupported/unavailable/partial/degraded/unknown
    计数。
  - TUI capability detail 只列 non-available entries，展示 capability id、status、backend、reason
    和可选 next step。
  - `tests/platform_layout.rs` 更新 renderer capability consumer 边界：允许 doctor 和 TUI Status
    消费，继续禁止 GUI/WebView/IPC/daemon client/task 进入 TUI summary。

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
- Phase 9o 已跑：`cargo test --test platform_layout gui_first_screen_request_plan_reuses_client_api_without_sending_ipc`，
  先红灯失败于缺少 `gui_first_screen_request_plan`，实现后通过。
- Phase 9o 已跑：`cargo test --test platform_layout`，通过 27 个测试。
- Phase 9o 已跑：`cargo check --manifest-path src-tauri/Cargo.toml`，通过。
- Phase 9o 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 9o 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  639 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、27 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 9o 已跑：`rg -n "tauri|wry|webview|WebView|tao" Cargo.toml src/daemon src/tui src/client_api.rs`，
  无命中。
- Phase 9o 已跑：`rg -n "connect_default|DaemonClient|send_command|subscribe_events|tokio::spawn|tokio::time|std::thread::spawn" src-tauri`，
  无命中。
- Phase 9p 已跑：`cargo test --test platform_layout gui_daemon_status_snapshot_shape_does_not_send_ipc`，
  先红灯失败于缺少 `gui_daemon_status_snapshot`，实现后通过。
- Phase 9p 已跑：`cargo check --manifest-path src-tauri/Cargo.toml`，通过。
- Phase 9p 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、28 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9q 已跑：`cargo test --manifest-path src-tauri/Cargo.toml daemon_status_event_maps_to_snapshot_shape_without_ipc`，
  先红灯失败于缺少 `gui_daemon_status_snapshot_from_event`，实现后通过。
- Phase 9q 已跑：`cargo test --test platform_layout gui_daemon_status_event_mapper_is_pure_and_local_to_tauri_app`，
  通过。
- Phase 9q 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml daemon_status_event_maps_to_snapshot_shape_without_ipc`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、29 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 `daemon_status_event_maps_to_snapshot_shape_without_ipc`。
- Phase 9r 已跑：`cargo test --test platform_layout gui_daemon_status_one_shot_request_is_explicit_and_bounded`，
  先红灯失败于缺少 `gui_daemon_status_request_once`，实现后通过。
- Phase 9r 已跑：`cargo test --manifest-path src-tauri/Cargo.toml daemon_status`，通过 2 个
  Tauri crate 单元测试。
- Phase 9r 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml daemon_status`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、30 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 2 个 daemon status tests。
- Phase 9s 已跑：`cargo test --test platform_layout gui_history_summary_one_shot_request_is_explicit_and_bounded`，
  先红灯失败于缺少 `gui_history_summary_request_once`，实现后通过。
- Phase 9s 已跑：`cargo test --manifest-path src-tauri/Cargo.toml history_summary`，通过 2 个
  Tauri crate 单元测试。
- Phase 9s 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml history_summary`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、31 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 2 个 history summary tests。
- Phase 9t 已跑：`cargo test --test platform_layout gui_first_screen_summary_one_shot_request_is_explicit_and_bounded`，
  先红灯失败于缺少 `gui_first_screen_summary_request_once`，实现后通过。
- Phase 9t 已跑：`cargo test --manifest-path src-tauri/Cargo.toml first_screen_summary`，通过 2 个
  Tauri crate 单元测试。
- Phase 9t 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_summary`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、32 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 2 个 first-screen summary tests。
- Phase 9u 已跑：`cargo test --test platform_layout gui_first_screen_summary_timing_stays_local_to_one_shot_request`，
  先红灯失败于缺少 `GuiFirstScreenSummaryTiming`，实现后通过。
- Phase 9u 已跑：`cargo test --manifest-path src-tauri/Cargo.toml first_screen_summary`，通过 2 个
  Tauri crate 单元测试，覆盖 first-screen summary timing 默认 shape 和 recoverable error shape。
- Phase 9u 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_summary`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、33 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 2 个 first-screen summary tests。
- Phase 9v 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_shape_is_static_and_explicit` 先红灯失败于缺少
  `gui_first_screen_refresh_shape`，实现后通过；`cargo test --manifest-path src-tauri/Cargo.toml first_screen_refresh_shape`
  通过 1 个 Tauri crate 单元测试。
- Phase 9v 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_refresh_shape`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、34 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen refresh shape test。
- Phase 9w 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_readiness_shape_is_static_display_preflight`
  先红灯失败于缺少 `gui_first_screen_readiness_shape`，实现后通过；
  `cargo test --manifest-path src-tauri/Cargo.toml first_screen_readiness_shape` 通过 1 个 Tauri crate
  单元测试。
- Phase 9w 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_readiness_shape`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、35 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen readiness shape test。
- Phase 9x 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_offline_shape_is_static_display_preflight`
  先红灯失败于缺少 `gui_first_screen_offline_shape`，实现后通过；
  `cargo test --manifest-path src-tauri/Cargo.toml first_screen_offline_shape` 通过 1 个 Tauri crate
  单元测试。
- Phase 9x 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_offline_shape`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、36 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen offline shape test。
- Phase 9y 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_command_policy_shape_keeps_one_shots_explicit`
  先红灯失败于缺少 `gui_first_screen_command_policy_shape`，实现后通过；
  `cargo test --manifest-path src-tauri/Cargo.toml first_screen_command_policy` 通过 1 个 Tauri crate
  单元测试。
- Phase 9y 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_command_policy`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、37 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen command policy test。
- Phase 9z 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_affordance_shape_stays_static`
  先红灯失败于缺少 `gui_first_screen_refresh_affordance_shape`，实现后通过；
  `cargo test --manifest-path src-tauri/Cargo.toml first_screen_refresh_affordance` 通过 1 个
  Tauri crate 单元测试。
- Phase 9z 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_refresh_affordance`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、38 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen refresh affordance shape test。
- Phase 9aa 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_click_wiring_is_explicit_only`
  先红灯失败于缺少 `refresh-action-button`，实现后通过。
- Phase 9aa 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、39 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ab 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_result_projection_stays_click_scoped`
  先红灯失败于缺少 `projectExplicitRefreshSummary`，实现后通过。
- Phase 9ab 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、40 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ac 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_error_projection_stays_catch_scoped`
  先红灯失败于缺少 `projectExplicitRefreshError`，实现后通过。
- Phase 9ac 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、41 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ad 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_success_clears_offline_display`
  先红灯失败于 success projection 未清理 `offline-problem-kind`，实现后通过。
- Phase 9ad 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、42 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ae 已跑窄验证：
  `cargo test --test platform_layout gui_frontend_invokes_are_authorized_and_init_errors_are_visible`
  先红灯失败于 `allow-gui-shell-metadata` 未授权；补 `src-tauri/permissions/gui.toml`、capability
  allow 列表和初始化错误投影后通过。
- Phase 9ae 已跑 Tauri 验证：`cargo check --manifest-path src-tauri/Cargo.toml`，先红灯失败于
  application permission 文件缺失，补 `src-tauri/permissions/gui.toml` 后通过。
- Phase 9ae 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、43 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9af 已跑窄验证：
  `cargo test --test platform_layout gui_static_frontend_global_tauri_api_is_enabled_and_missing_api_is_visible`
  先红灯失败于 `src-tauri/tauri.conf.json` 未启用 `withGlobalTauri`；补配置和 missing API 错误显示后通过。
- Phase 9af 已跑 Tauri 验证：`cargo check --manifest-path src-tauri/Cargo.toml`，通过。
- Phase 9af 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、44 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ag 已跑窄验证：
  `cargo test --test platform_layout gui_manual_refresh_summary_is_readable_and_click_scoped`
  先红灯失败于缺少 `manual-summary-status`；补静态 summary 字段和 success/error projection 后通过。
- Phase 9ag 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、45 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ah 已跑窄验证：
  `cargo test --test platform_layout gui_frontend_first_screen_view_model_is_local_preflight_only`
  先红灯失败于缺少 `firstScreenViewModel`；补本地 view model 和 projection helper 后通过。
- Phase 9ah 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、46 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ai 已跑窄验证：
  `cargo test --test platform_layout gui_backend_event_stream_start_is_tauri_owned_and_explicit`
  先红灯失败于缺少 backend event stream command；补 Tauri-owned explicit stream command 后通过。
- Phase 9ai 已跑 Tauri 验证：`cargo check --manifest-path src-tauri/Cargo.toml`，通过。
- Phase 9ai 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、47 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9aj 已跑窄验证：
  `cargo test --test platform_layout gui_frontend_daemon_event_listener_wiring_is_event_only`
  先红灯失败于缺少 `window.__TAURI__.event.listen`；补 frontend listener、stream start 和 event projection 后通过。
- Phase 9aj 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、48 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ak 已跑窄验证：
  `cargo test --test platform_layout gui_backend_event_stream_forwards_recording_state_changes`
  先红灯失败于缺少 `Event::StateChanged` mapper；补 mapper 后用户验证仍失败；强化测试要求 stream
  loop 不再用 shared first-screen classifier 过滤，改由 `gui_daemon_event_payload()` 直接决定 emit 后通过。
- Phase 9ak 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、49 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9al 已跑窄验证：
  `cargo test --test platform_layout gui_event_stream_projects_first_screen_data_without_refresh`
  先红灯失败于缺少 live stats/text/history appended projection；补 backend payload 和 frontend projection 后通过。
- Phase 9al 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、50 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 7b/8b 已跑窄验证：
  `cargo test --test platform_layout overlay_windows_linux_backend_skeletons_are_cfg_gated_and_gui_free`
  先红灯失败于缺少 `src/overlay/windows.rs`，补 Windows/Linux cfg-gated backend skeleton 后通过。
- Phase 7b/8b 已跑：`cargo test overlay::renderer::tests`，通过 3 个 renderer 单元测试。
- Phase 7b/8b 已跑 cross target check：
  `cargo check --target x86_64-pc-windows-msvc` 被既有 Unix-only `src/ipc/transport.rs` 阻断；
  `cargo check --target x86_64-unknown-linux-gnu` 被 OpenSSL cross sysroot 阻断。这不是 overlay
  skeleton 自身的完整非 macOS 编译证明，需后续 IPC transport / Linux build 环境阶段解决。
- Phase 7b/8b 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、51 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 3b IPC transport cfg boundary 已跑窄验证：
  `cargo test --test platform_layout ipc_transport_backends_are_cfg_gated` 先红灯失败于 transport 未 cfg-gate，
  补 `src/ipc/transport.rs` Unix/Windows backend skeleton 后通过。
- Phase 3b 已跑：`cargo test ipc::transport::tests`，通过 3 个 Unix UDS transport 测试。
- Phase 3b 已跑：`cargo test platform::lifecycle`，通过 2 个 Unix lifecycle 测试。
- Phase 3b 已跑：`cargo check --target x86_64-pc-windows-msvc`，exit 0；仍有大量 dead-code/unused
  warning，原因是 Windows backend 多数仍是 unsupported skeleton，后续不能把它等同于 Windows 可运行。
- Phase 3b 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、52 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 10a 已跑：`cargo test cli::doctor::tests`，通过。
- Phase 10a 已跑：`cargo test --test platform_layout network_clients_use_rustls_for_cross_platform_checks`，
  通过。
- Phase 10a 已跑：`cargo fmt --check`，通过。
- Phase 10a 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 10a 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  640 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、53 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 10a 已跑：`make check-windows`，exit 0；仍有大量 dead-code/unused warning，原因是
  Windows backend 多数仍是 unsupported skeleton，不能等同于 Windows 可运行。
- Phase 10a 已跑：`make check-linux`，失败于缺少 `x86_64-linux-gnu-gcc` / Linux sysroot；
  已越过 OpenSSL/native-tls 阻断，当前是本机 cross toolchain 环境问题。
- Phase 10b 已跑窄验证：
  `cargo test tui::status::tests::platform_capability_lines_include_problem_details` 先红灯失败于缺少
  `platform_capability_lines`，实现后通过。
- Phase 10b 已跑：`cargo test --test platform_layout`，通过 54 个测试。
- Phase 10b 已跑：`cargo fmt --check`，通过。
- Phase 10b 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 10b 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  641 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、54 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 3c 已跑窄验证：
  `cargo test --test platform_layout windows_ipc_transport_uses_tokio_named_pipe_backend` 先红灯失败于
  Windows IPC transport 仍是 placeholder，改为 Tokio Named Pipe backend 后通过。
- Phase 3c 已跑：`cargo fmt --check`，通过。
- Phase 3c 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 3c 已跑：`cargo test --test platform_layout`，通过 55 个测试。
- Phase 3c 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  641 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、55 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 3c 已跑：`make check-windows`，exit 0；仍有大量 dead-code/unused warning，原因是
  Windows hotkey/overlay/service/lifecycle 等 backend 仍多为 skeleton，不能等同于 Windows runtime 可用。
- macOS 权限、录音、overlay、clipboard/paste、TUI、service lifecycle、history 手动体验：未执行，
  需用户在真实 macOS 会话按 `macos-baseline.md` checklist 验证。

## 已知风险

- `src/cli/doctor.rs` 仍有 launchd-centric 诊断输出；service manager facade 后应通过
  capability/status 和 service manager 模型收敛。
- Phase 5b 只抽 hotkey provider 启动边界，没有实现 Linux/Windows global hotkey backend。
- Phase 10b 只把 renderer/platform capability snapshot 接入 TUI Status 静态摘要；Phase 7b/8b
  已有 Windows/Linux overlay backend skeleton，但还没有真实 renderer 实现。
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
- Phase 9m/9n/9o/9p/9q/9r/9s/9t/9u/9v/9w/9x/9y/9z/9aa/9ab/9ac/9ad/9ae/9af/9ag/9ah/9ai/9aj/9ak/9al 只创建最小 `src-tauri/**` skeleton、静态 placeholder、本地 metadata
  command、first-screen request plan command、daemon status snapshot shape command、纯 daemon
  status event mapper、显式 one-shot daemon status request command 和显式 one-shot history summary
  request command、显式 one-shot first-screen summary request command、first-screen summary 本地
  timing 字段、first-screen explicit refresh shape、first-screen readiness/timing display shape 和
  first-screen offline/error display shape、first-screen command invocation policy shape、
  first-screen explicit refresh affordance shape、placeholder explicit refresh click wiring 和
  click-scoped summary/error projection、success offline clear、application command ACL、初始化错误可见性和静态
  frontend global Tauri API、手动 Refresh 可读摘要、本地 first-screen view model 和显式 backend
  daemon event stream bridge、frontend daemon event listener wiring、`StateChanged` forwarding 和
  first-screen stream data projection；
  尚未运行 `tauri dev` / `tauri build` / `tauri bundle`，也没有启动 GUI 或 daemon。后续需要
  单独决定何时运行 release build、如何记录 cold start/RSS/CPU/bundle 指标。
- Phase 9n 的 `gui_shell_metadata` 只验证本地 command wiring，不连接 daemon、不读
  config/history、不生成真实 Status/History/Diagnostics view model。
- Phase 9o 的 `gui_first_screen_request_plan` 只生成请求计划 summary，不发送 IPC、不订阅
  event stream、不读取 daemon status。
- Phase 9p 的 `gui_daemon_status_snapshot` 只固定 status response shape，不连接 daemon、
  不发送 `Command::DaemonStatus`、不读取真实 `Event::DaemonStatus`。
- Phase 9q 的 `gui_daemon_status_snapshot_from_event` 只映射调用方已提供的
  `Event::DaemonStatus`；仍没有真实 IPC request、event stream 或 Tauri event emission。
- Phase 9r 的 `gui_daemon_status_request_once` 只做显式 one-shot request；不自动调用、不订阅、
  不重连、不启动 daemon、不提供 service management。
- Phase 9s 的 `gui_history_summary_request_once` 只做显式 one-shot request；不自动调用、不订阅、
  不重连、不启动 daemon、不提供完整 History view model。
- Phase 9t 的 `gui_first_screen_summary_request_once` 只做显式 one-shot request；不自动调用、
  不订阅、不重连、不启动 daemon、不提供 frontend Status/History view model。
- Phase 9u 的 first-screen summary timing 只描述本次显式 request 的 GUI backend 本地耗时；
  不代表 daemon 内部状态，不写入 protocol/history/trace。
- Phase 9v 的 `gui_first_screen_refresh_shape` 只描述后续前端手动刷新入口的静态 shape；
  placeholder 不自动调用 `gui_first_screen_summary_request_once`，也不实现 loading/retry UI。
- Phase 9w 的 `gui_first_screen_readiness_shape` 只描述 placeholder 首屏 readiness/timing 空态；
  不读取真实 daemon event、不调用 one-shot request、不启动 timer 或 metrics sink。
- Phase 9x 的 `gui_first_screen_offline_shape` 只描述 placeholder 首屏 daemon offline /
  recoverable error 空态；不启动 daemon、不安装/重启 service、不启动 reconnect loop。
- Phase 9y 的 `gui_first_screen_command_policy_shape` 只描述 placeholder 可自动调用的静态
  command 和必须显式触发的 one-shot command；不作为真实 command dispatcher。
- Phase 9z 的 `gui_first_screen_refresh_affordance_shape` 只描述 placeholder 手动刷新控件的
  静态展示字段；不注册真实 click handler，不自动调用 one-shot request。
- Phase 9aa 的 placeholder refresh button 只在用户 click 后调用既有
  `gui_first_screen_summary_request_once`；初始加载不自动请求，不订阅、不重连、不启动 daemon。
- Phase 9ab 的 `projectExplicitRefreshSummary` 只在 explicit refresh click 成功路径内把 summary
  投影到现有 placeholder 文本字段；不新增 backend command，不建立完整 view model。
- Phase 9ac 的 `projectExplicitRefreshError` 只在 explicit refresh click catch 路径内把 request
  error 投影到现有 placeholder 文本字段；不新增 backend command，不实现 retry loop。
- Phase 9ad 的 `projectExplicitRefreshSummary` 只在 explicit refresh click success 路径内清理
  stale offline/error 文本；不新增 backend command，不新增请求。
- Phase 9ae 只保证当前 placeholder frontend invoke 的 Tauri application commands 被 capability
  授权，并且初始化失败不再静默吞掉；不实现 daemon event subscription、recording state streaming、
  reconnect supervisor 或自动首屏 one-shot。
- Phase 9af 只为无 bundler 静态 HTML 启用 `withGlobalTauri`，并在 `window.__TAURI__` API 缺失时显示
  `tauri-api-missing`；不实现 daemon event subscription、recording state streaming、reconnect
  supervisor 或自动首屏 one-shot。
- Phase 9ag 只在 explicit Refresh success/catch 路径更新 manual summary 文本；不实现 daemon event
  subscription、recording state streaming、reconnect supervisor 或自动首屏 one-shot。
- Phase 9ah 只在静态 HTML 内维护本地 `firstScreenViewModel`；不实现 daemon event subscription、
  recording state streaming、reconnect supervisor 或自动首屏 one-shot。
- Phase 9ai 只在 Tauri backend 暴露显式 `gui_start_daemon_event_stream` command 并启动
  GUI-owned event stream task；不实现 reconnect supervisor、daemon auto-start 或 service management。
- Phase 9aj 只在 frontend 初始化时注册 Tauri event listener 并显式启动 event stream bridge；
  event payload 只投影到 placeholder view model/DOM，不提供 start/stop/cancel recording controls、
  reconnect supervisor、window close cancellation 或完整 Status/History view。
- Phase 9ak 只修复 backend stream mapper，把既有 `StateChanged` 转成现有 `daemonStatus`
  payload，并移除 stream loop 对 shared first-screen classifier 的前置过滤；不新增 IPC event、不改变
  daemon/TUI 行为、不新增 GUI recording controls。
- Phase 9al 只把既有 `StatsChanged`、`Partial`、`Segment`、`HistoryAppended` 投影到现有
  placeholder 字段；不自动触发 Refresh、不建立完整 History view、不新增 IPC event 或 polling。
- GUI PoC 冻结：`src-tauri/**` 和 `gui-dist/index.html` 只保留为未来 GUI 接口验证成果；不要继续
  打磨 placeholder 页面，不实现 reconnect supervisor、recording controls、service management、
  配置编辑器或 release/bundle 指标，除非重新进入 GUI 产品设计阶段。
- Phase 7b/8b overlay skeleton 已开始：`src/overlay/windows.rs` 和 `src/overlay/linux.rs`
  作为 cfg-gated backend skeleton，`overlay::renderer` 在 Windows/Linux 下调度到对应 backend。
  Windows 当前报告 `win32_overlay_skeleton` structured unsupported；Linux 当前报告
  `wayland_overlay_skeleton`，其中 window anchor 为 `degraded/screen_anchor_expected`。
- `ipc::transport` 已有 Windows Named Pipe compile backend，但未在 Windows 实机/VM 验证 runtime
  connect/bind/accept、ACL/security descriptor、multi-user 隔离或 pipe busy 行为。Windows daemon
  lock/process probe/smart fallback 同样仍只是 unsupported skeleton。
- `current_platform_capabilities()` 是 Phase 1 静态快照，不执行权限 probe；后续消费方不要把
  静态 `desktop.permissions=available` 误解为当前已授权。
- `overlay::renderer::renderer_capabilities()` 同样是静态快照，不创建窗口、不 probe 当前
  compositor/权限、不读取业务配置。

## 下一步

Phase 3c Windows Named Pipe transport compile backend 已完成一个最小阶段。下一步：

- 下一阶段若继续 Windows，可做 Windows daemon single-instance/process probe/smart fallback skeleton
  收敛，或继续把 desktop/hotkey/service 的 Windows unsupported skeleton 接入 doctor/TUI 诊断。
- 若继续 Linux build baseline，应优先配置 Linux C cross compiler/sysroot，或改用 Docker/cross/CI。
- 若继续 overlay 视觉 PoC，则需要用户提供真实 Windows 11/10 或 Linux wlroots/KDE/GNOME 环境；
  在当前 macOS 主机上不要假装验证真实 topmost/click-through/layer-shell 行为。
- 真实 Windows 11/10、wlroots/KDE/GNOME overlay 视觉验证需要用户后续提供目标系统环境。
- 不继续 GUI 产品化开发。

建议下一 session prompt：

```text
继续 /Users/ghot/repo/shuohua 跨平台改造，当前在 feat/cross-platform-design。
先读 AGENTS.md、TODO、docs/cross-platform/README.md、overview.md、
development-plan.md、gui.md、overlay.md、platform-capabilities.md、macos-baseline.md、
handoff.md。
Phase 9al 后 GUI PoC 已冻结；不要继续打磨 GUI placeholder。
Phase 7b/8b overlay backend skeleton、Phase 3b IPC transport cfg boundary、Phase 10a
cross-check baseline、Phase 10b TUI capability diagnostics、Phase 3c Windows Named Pipe
transport compile backend 已完成一个最小阶段。先查看最新 diff/commit 和验证结果。
保持 macOS 不回退，不引入 GUI/WebView。不要把 Windows Named Pipe compile backend 当成实机
runtime 验收。下一步优先考虑 Windows lifecycle/smart fallback 诊断收敛或 Linux Docker/cross
build baseline；真实 overlay 视觉 PoC 需要用户提供目标系统。
```
