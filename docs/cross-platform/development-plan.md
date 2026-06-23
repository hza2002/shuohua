# Cross-Platform Development Plan

## 原则

跨平台改造必须小步推进。每个阶段只改变一个边界，先保护 macOS 行为，再加入新平台能力。

每个阶段的默认流程：

1. 更新对应设计文档，写清当前判断、风险和验收。
2. 写最小测试或检查，先证明现状/目标边界。
3. 做最小实现，不顺手重构无关模块。
4. 跑受影响测试。
5. 若触及共享边界，跑 `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`。
6. 手动验证项不能由自动测试替代时，在提交说明里明确未手动验证的范围。

macOS 当前可用版本是回归基线。任何阶段如果破坏 macOS hotkey、录音、ASR、post、clipboard、
paste、overlay、TUI、history 或 service lifecycle，应先回滚或修复该阶段，再继续。

## Phase 0: Baseline Audit

目标：建立重构前基线，避免后续不知道是否回退。

范围：

- 记录当前 macOS 关键路径。
- 补齐缺失的 platform-boundary 测试。
- 审计配置字段、平台 facade、macOS-only import。

验收：

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- 用户手动验证 macOS 录音、停止、取消、overlay、上屏、TUI/service。

## Phase 1: Platform Capability Model

目标：先统一表达能力状态，不改变现有行为。

范围：

- 新增共享 capability/status 类型。
- doctor/TUI 可以读取但不强依赖。
- macOS 现有能力先映射为 current status。
- 非 macOS 先返回 structured unsupported。

不做：

- 不抽 hotkey backend。
- 不改 IPC protocol。
- 不新增 Linux/Windows 实现。

验收：

- macOS 行为不变。
- unsupported/partial/degraded/unavailable 能被测试覆盖。
- doctor 输出不退化。

## Phase 2: Config And Theme Cross-Platform Rules

目标：让配置和 theme 具备平台扩展空间，但不破坏现有配置。

范围：

- 明确通用字段、平台段、metadata、advanced 字段规则。
- 降低实验字段在 starter config 中的存在感。
- 为 overlay material preference 增加 schema 设计前评审。

不做：

- 不马上改所有 theme。
- 不引入 GUI 配置编辑器。

验收：

- 现有用户配置继续 parse。
- 官方模板字段都有 schema 和使用点或 metadata 说明。
- unknown typo 仍被诊断。

## Phase 3: IPC Transport Boundary

目标：协议不变，transport 可替换。

范围：

- 抽 client/server transport facade。
- macOS 继续 Unix domain socket。
- Linux 可复用 Unix domain socket。
- Windows 只设计 Named Pipe adapter，不急于完整实现。

不做：

- 不改 JSON-line command/event shape。
- 不改 history schema。

验收：

- 现有 IPC tests 通过。
- TUI 对 transport 细节无感。
- stale socket 清理仍只在 daemon lock owner 路径。

## Phase 4: Single Instance, Process Probe, Service Manager

目标：把 daemon 生命周期从 macOS launchd 细节中拆出来。

范围：

- 抽单实例 lock facade。
- 抽 process probe。
- 抽 service manager facade。
- macOS launchd backend 保持行为。
- Linux systemd user、Windows logon task 先做设计或 dry-run shell。

不做：

- 不自动安装 Linux/Windows service。
- 不改变 `shuo app service` 用户可见语义。

验收：

- macOS service install/start/stop/restart/status 行为不变。
- CLI runtime boundary 不回退。
- stop timeout 和 shutdown ack 语义不变。

## Phase 5: Desktop Capability Boundary

目标：拆 hotkey、clipboard、text injection、active app、permissions 的平台边界。

范围：

- 保留 macOS CGEventTap 实现。
- 业务层只依赖 platform facade。
- 非 macOS 返回 capability-aware unsupported。
- 为 Windows/Linux backend 留小接口，不提前扩大。

不做：

- 不在第一步实现全局 hotkey。
- 不引入大型跨平台输入库，除非 PoC 证明必要。

验收：

- macOS suppress down/up 配对测试不回退。
- clipboard/paste 行为不变。
- profile route 仍使用 frontmost app 信息。

## Phase 6: Overlay Renderer Boundary

目标：共享 overlay model/layout/theme，renderer 平台化。

范围：

- 保留 macOS AppKit renderer。
- 抽 renderer availability/capability。
- 建立 Windows/Linux renderer 骨架。
- 确认 material fallback：liquid_glass -> blurred_glass -> translucent -> solid。

不做：

- 不一次性实现 Windows/Linux 完整 overlay。
- 不让 renderer 直接读取业务配置。

验收：

- macOS overlay 视觉和状态机不回退。
- `OverlayCmd`、`OverlayModel`、layout tests 通过。
- capability 能表达材质和窗口能力降级。

## Phase 7: Windows Overlay PoC

目标：验证 Windows 原生 overlay 技术路线。

Phase 7a 先做文档化 PoC baseline，不写 backend：

- 基于 Microsoft 文档确认 Win32 window style、layered alpha、topmost/no-activate/tool
  window、hit-test 穿透和 Mica/DWM backdrop 的候选路径。
- 把 Windows 11 / Windows 10 的实测 checklist 写入 `overlay.md`。
- 不新增 Windows renderer 文件，不引入 windows crate，不改变 macOS overlay。

范围：

- 验证 Win32 topmost/layered/tool window。
- 验证 DWM/Mica/Acrylic 或 fallback。
- 验证文字绘制、show/hide 延迟、鼠标穿透。

验收：

- 记录 Windows 11 结果。
- 记录 Windows 10 fallback。
- 若成本过高，修订 overlay 文档而不是硬做。

## Phase 8: Linux Wayland Overlay PoC

目标：验证 Wayland-first overlay 可用性。

Phase 8a 先做文档化 PoC baseline，不写 backend：

- 基于 Wayland core/xdg-shell、wlr layer-shell、KDE/GTK layer-shell 和 GNOME Mutter
  公开资料确认 compositor 约束。
- 把 wlroots/KDE/GNOME/X11 的实测 checklist 写入 `overlay.md`。
- 不新增 Linux renderer 文件，不引入 Wayland crate，不改变 macOS overlay。

范围：

- 验证主流 GNOME/KDE Wayland 能力。
- 验证 layer-shell 或可替代方案。
- 验证无法置顶/穿透/锚定时的 solid fallback。

验收：

- 核心状态和文本可读。
- 能力不足被清楚诊断。
- X11 backend 是否需要实现有明确结论。

## Phase 9: Tauri GUI Client PoC

目标：验证 GUI 作为独立 client 的开销和集成成本。

Phase 9a 先做文档化 PoC baseline，不写 GUI app：

- 基于 Tauri v2 文档确认 command/event、permissions/capabilities、sidecar、
  build/bundle 和 release 指标采集边界。
- 把进程边界、IPC、安全权限、指标、打包和 TUI 回退 checklist 写入 `gui.md`。
- 不新增 Tauri workspace，不引入 WebView runtime，不改变 daemon/CLI/TUI。

Phase 9b 先做 GUI client API boundary，不写 GUI app：

- 增加共享 daemon client API 边界，复用现有 JSON-line IPC command/event。
- TUI 开始通过该边界引用 daemon client 类型，后续 GUI backend 也走同一入口。
- 增加架构测试，禁止 daemon/TUI/shared client API 引入 Tauri、WRY 或 WebView 依赖/token。
- 不新增 Tauri workspace，不新增 wire protocol，不改变 TUI 用户可见行为。

Phase 9c 扩展 GUI 首屏 client helper，不写 GUI app：

- 在 shared client API 中增加首屏 request helper：subscribe、daemon status、history page、
  history stats。
- 增加 response classifier，把现有 daemon `Event` 分类为 GUI backend 可消费的首屏输入。
- 不新增 IPC command/event，不新增 Tauri workspace，不读取 history/config 文件。

Phase 9d 记录 GUI library boundary 前置条件，不写 GUI app：

- 明确当前 crate 只有 binary target，`client_api` 不是外部 crate 可复用的 library API。
- 记录创建 Tauri workspace 前必须先做 library split 评审。
- 增加架构测试，防止当前阶段误加 `src/lib.rs`、Tauri workspace 或 WebView runtime 依赖。

Phase 9e 记录 library split audit baseline，不写 GUI app：

- 审计 `client_api` / `ipc::client` / `ipc::protocol` / `ipc::transport` 的最小可复用 surface。
- 记录 `ipc::protocol` 对 `history` / `state` 模型的依赖，和 `ipc::transport` 的 Unix-only
  transport 现状。
- 不创建 `src/lib.rs`，不创建 Tauri workspace，不移动核心文件。

Phase 9f 做最小 library split，不写 GUI app：

- 新增 library target，让后续外部 GUI backend 可依赖 `client_api`、`ipc::client`、
  `ipc::protocol`、`ipc::transport` 和必要 DTO。
- binary 继续拥有 daemon、CLI/TUI、platform backend、overlay、voice、hotkey、reload/config
  和 IPC server。
- 不新增 IPC protocol，不创建 Tauri workspace，不新增 WebView runtime，不抽 Windows Named Pipe。

Phase 9g 增加 GUI client 连接状态骨架，不写 GUI app：

- 在 shared `client_api` 中增加 daemon connection state、recoverable problem kind 和有上限的
  retry delay helper。
- 不实现后台 reconnect task，不改变 TUI 行为，不自动启动 daemon。
- 不新增 IPC protocol，不创建 Tauri workspace，不新增 WebView runtime。

Phase 9h 增加 GUI backend event bridge 骨架，不写 GUI app：

- 在 shared `client_api` 中增加 GUI backend event 类型，把 daemon event、connection state 和
  recoverable connection problem 统一成可转发事件。
- bridge 只做纯分类和封装，复用既有首屏 event classifier。
- 不新增 IPC protocol，不创建 Tauri workspace，不新增 WebView runtime，不生成 frontend
  view model。

Phase 9i 增加 GUI 首屏 metrics/timing 纯模型，不写 GUI app：

- 在 shared `client_api` 中增加首屏 timing/readiness 类型，用于后续 GUI backend 采集 GUI
  启动、daemon connect、首个 daemon event 和首屏 ready 耗时。
- helper 只使用调用方传入的毫秒时间戳和既有 `FirstScreenEvent`，不读系统时间、不启动
  runtime、不连接 IPC、不写 metrics sink。
- 不新增 IPC protocol，不创建 Tauri workspace，不新增 WebView runtime，不生成 frontend
  view model。

Phase 9j 记录 Tauri permissions/capabilities preflight，不写 GUI app：

- 基于 Tauri v2 文档记录 capabilities/permissions 如何限制 windows/webviews 可访问的
  command/plugin。
- 明确 PoC 只允许主 window/webview 使用最小 shuohua GUI backend command，不默认开启
  shell/filesystem/http/process/global shortcut/updater/sidecar 等宽权限。
- 不新增 IPC protocol，不创建 Tauri workspace，不新增 WebView runtime，不生成 frontend
  view model。

Phase 9k 记录最小 Tauri workspace 创建前验收清单，不写 GUI app：

- 明确下一阶段创建 workspace 时允许新增的最小文件范围、禁止的 scope creep、自动验收和
  release 指标采集清单。
- 基于 Tauri v2 build/bundle 文档记录 PoC 指标必须来自 release build 和 bundle 产物，而不是
  dev server。
- 不新增 IPC protocol，不创建 Tauri workspace，不新增 WebView runtime，不运行 Tauri build
  或 bundle。

Phase 9l 记录 GUI daemon offline/reconnect 后台任务 ownership，不写 GUI app：

- 明确后续 GUI backend 连接 supervisor task 的所有权、取消 owner、session generation、
  recoverable problem 范围和 metrics ownership。
- shared `client_api` 继续只提供纯状态/退避/event bridge/timing helper，不拥有 spawn、timer、
  channel、Tauri event emission 或 metrics sink。
- 不新增 IPC protocol，不创建 Tauri workspace，不新增 WebView runtime，不实现 runtime loop。

Phase 9m 创建最小 Tauri workspace skeleton，不接 daemon、不实现页面：

- 新增 `src-tauri/**` 最小标准骨架和主 window capability，让后续 GUI backend 有独立 crate。
- Tauri/WRY/WebView runtime 只允许出现在 `src-tauri/**`；root crate、daemon、TUI 和
  shared `client_api` 不引入 GUI runtime。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不实现 reconnect loop
  或 frontend view model。

Phase 9n 增加最小 GUI backend shell 和静态 frontend placeholder，不接 daemon：

- `src-tauri` 可以注册一个本地 metadata command，用于验证 Tauri command wiring 和 frontend
  invoke 入口；command 只能返回静态 GUI shell 信息，不连接 IPC、不读配置/history。
- 新增最小 `gui-dist/**` 静态 placeholder，让 `frontendDist` 有可审计输入；不引入 npm/vite、
  build script、frontend dependency 或完整页面。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不实现 reconnect loop、
  service management、配置编辑或 history view model。

Phase 9o 增加 GUI first-screen request plan command，不发送 IPC：

- `src-tauri` 可以注册一个 first-screen request plan command，复用
  `shuohua::client_api::first_screen_commands()` 生成 GUI 首屏将要发送的既有 IPC command summary。
- command 只能返回 command kind、history limit 和是否需要 daemon connection 等静态计划信息；
  不创建 `DaemonClient`、不调用 `connect_default()`、不发送 IPC、不订阅 event stream。
- frontend placeholder 可以展示该 plan，但不得实现 Status/History/Diagnostics view model。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不实现 reconnect loop。

Phase 9p 增加 GUI daemon status snapshot shape command，不发送 IPC：

- `src-tauri` 可以注册一个 daemon status snapshot command，用于固定后续真实 status view 的
  response shape。该 command 只能描述当前 GUI backend 未连接 daemon、transport 未打开，以及
  后续需要发送既有 `Command::DaemonStatus`。
- command 返回静态字段和 request summary，不创建 `DaemonClient`、不调用 `connect_default()`、
  不发送 IPC、不读取 daemon status event、不启动 reconnect loop。
- frontend placeholder 可以展示 status snapshot shape，但不得实现真实 Status/History/Diagnostics
  view model，不读 config/history 文件，不直接访问 IPC transport。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不实现 service
  management。

Phase 9q 增加 GUI daemon status event mapper，不发送 IPC：

- `src-tauri` 可以增加一个纯 mapper，把既有 `Event::DaemonStatus` 映射到 Phase 9p 固定的
  daemon status snapshot response shape。
- mapper 只能消费已由调用方传入的 event；不得创建 `DaemonClient`、不得调用
  `connect_default()`、不得调用 `send_command`、不得订阅 daemon event stream、不得启动
  reconnect loop 或 timer。
- 现有 `gui_daemon_status_snapshot` command 继续返回未连接静态 shape，不读取真实 event。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不实现 service
  management 或 frontend view model。

Phase 9r 增加 GUI daemon status one-shot request command，不订阅、不重连：

- `src-tauri` 可以注册一个 one-shot daemon status request command：连接现有 daemon IPC，
  发送既有 `Command::DaemonStatus`，等待一个既有 `Event::DaemonStatus`，并复用 Phase 9q mapper
  返回 status snapshot shape。
- command 只允许在用户/前端显式调用时运行；placeholder 不自动调用它，避免打开 GUI 时默认
  连接 daemon。
- 连接失败、读写失败、daemon 返回 `Event::Error` 或连接提前关闭时，返回 recoverable
  request error shape；不得自动启动 daemon、不得安装/重启 service。
- 不订阅 daemon event stream，不启动 reconnect loop/timer，不新增 IPC protocol，不运行
  Tauri dev/build/bundle，不启动 daemon/GUI，不实现 frontend Status view model。

Phase 9s 增加 GUI history summary one-shot request command，不订阅、不重连：

- `src-tauri` 可以注册一个 one-shot history summary request command：连接现有 daemon IPC，
  发送既有 `Command::GetHistory { limit, before: None, before_id: None, query: None }` 和
  `Command::GetHistoryStats`，等待既有 `Event::History` 与 `Event::HistoryStats`，返回最小首屏
  history summary shape。
- command 只允许在用户/前端显式调用时运行；placeholder 不自动调用它，避免打开 GUI 时默认
  连接 daemon 或读取 history。
- summary 只包含 page count、matched、page aggregate stats、stats snapshot aggregate、latest
  record summary 和 request metadata；不得实现完整 History view model、搜索、详情、audio
  管理、图表或本地化。
- 连接失败、读写失败、daemon 返回 `Event::Error` 或连接提前关闭时，返回 recoverable
  request error shape；不得自动启动 daemon、不得安装/重启 service。
- 不订阅 daemon event stream，不启动 reconnect loop/timer，不新增 IPC protocol，不运行
  Tauri dev/build/bundle，不启动 daemon/GUI，不实现 frontend History view model。

Phase 9t 增加 GUI first-screen summary one-shot request command，不订阅、不重连：

- `src-tauri` 可以注册一个 one-shot first-screen summary request command：打开一次现有 daemon
  IPC 连接，发送既有 `Command::DaemonStatus`、
  `Command::GetHistory { limit, before: None, before_id: None, query: None }` 和
  `Command::GetHistoryStats`，等待既有 `Event::DaemonStatus`、`Event::History` 与
  `Event::HistoryStats`，返回组合首屏 summary shape。
- command 只允许在用户/前端显式调用时运行；placeholder 不自动调用它，避免打开 GUI 时默认
  连接 daemon 或读取 history。
- summary 只组合 status snapshot、history summary 和 request metadata；不得实现 frontend
  view model、loading/retry UI、metrics 展示、event stream、搜索、详情、audio 管理或本地化。
- 连接失败、读写失败、daemon 返回 `Event::Error` 或连接提前关闭时，返回 recoverable
  request error shape；不得自动启动 daemon、不得安装/重启 service。
- 不订阅 daemon event stream，不启动 reconnect loop/timer，不新增 IPC protocol，不运行
  Tauri dev/build/bundle，不启动 daemon/GUI，不实现 frontend Status/History view model。

Phase 9u 增加 GUI first-screen summary request timing，不订阅、不重连：

- `gui_first_screen_summary_request_once` 可以在 response 上附带本次显式 request 的本地 timing：
  `connectDurationMs`、`firstEventMs`、`readyMs` 和 `requestDurationMs`。
- timing 只由 `src-tauri` command 使用 `std::time::Instant` 计算；不进入 daemon protocol、
  shared `client_api`、history、trace 或 metrics sink。
- 不使用 `tokio::time`，不启动 timer task，不订阅 daemon event stream，不实现 reconnect loop、
  loading/retry UI 或 frontend view model。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不改变 TUI/CLI 行为。

Phase 9v 增加 GUI first-screen explicit refresh shape，不自动请求：

- `src-tauri` 可以注册一个 refresh shape command，固定后续手动刷新入口的静态契约：explicit trigger、
  default history limit、requires daemon connection、transport not opened 和 invoke target。
- command 不创建 `DaemonClient`、不调用 `connect_default()`、不发送 IPC、不调用 one-shot summary、
  不订阅 event stream、不启动 reconnect loop/timer。
- placeholder 只展示 refresh shape，不自动调用真实 first-screen summary request，不实现 loading/retry UI
  或 frontend Status/History view model。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不改变 TUI/CLI 行为。

Phase 9w 增加 GUI first-screen readiness/timing display shape，不自动请求：

- `src-tauri` 可以注册一个 readiness/timing display shape command，固定后续首屏空态展示契约：
  `ready=false`、必需输入均未到达、timing 暂不可用、source 为 placeholder。
- command 不创建 `DaemonClient`、不调用 `connect_default()`、不发送 IPC、不调用 one-shot summary、
  不读系统时间、不启动 timer/reconnect loop、不做 Tauri event emission。
- placeholder 只展示 readiness/timing 空态字段，不实现 loading/retry UI 或 frontend Status/History
  view model。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不改变 TUI/CLI 行为。

Phase 9x 增加 GUI first-screen offline/error display shape，不自动恢复：

- `src-tauri` 可以注册一个 offline/error display shape command，固定后续首屏 daemon offline 和
  recoverable request error 的静态展示契约：connected=false、problem kind、recoverable、
  retry allowed、auto start/service management disabled。
- command 不创建 `DaemonClient`、不调用 `connect_default()`、不发送 IPC、不调用 one-shot summary、
  不启动 daemon、不安装/重启 service、不启动 timer/reconnect loop、不做 Tauri event emission。
- placeholder 只展示 offline/error 静态字段，不实现真实 retry button、loading state 或 frontend
  Status/History view model。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不改变 TUI/CLI 行为。

范围：

- 建一个最小 Tauri app。
- 连接 daemon IPC。
- 展示 status snapshot 和 history summary。
- 测量冷启动、空闲内存、空闲 CPU、包体。

不做：

- 不替代 TUI。
- 不加入 daemon 常驻路径。

验收：

- GUI 关闭后 daemon 继续运行。
- daemon 未打开 GUI 时不加载 WebView。
- 记录三端 PoC 指标。

## Phase 10: First Non-macOS Core Backend

目标：在不依赖完整 overlay/GUI 的前提下，让核心能力在第一个非 macOS 平台可运行。

建议顺序：

1. Linux cloud ASR core：config、ASR provider、post、history、IPC。
2. Linux service manager。
3. Linux desktop capability。
4. Windows core。

选择 Linux first 是为了更快复用 Unix socket 和 CI；Windows 设计约束必须在接口评审时同时考虑。

## 持续维护

- 每完成一个 phase，更新 `overview.md` 的阶段状态。
- 发现文档假设错误，先改文档，再改实现。
- 不把 PoC 临时日志放进长期文档；只记录结论、风险和决策。
