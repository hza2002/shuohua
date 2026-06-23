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

Phase 9y 增加 GUI first-screen command invocation policy shape，不自动请求：

- `src-tauri` 可以注册一个 command invocation policy shape command，固定 placeholder 阶段允许自动调用
  的静态 commands，以及必须用户显式触发的 one-shot request commands。
- command 不创建 `DaemonClient`、不调用 `connect_default()`、不发送 IPC、不调用 one-shot summary、
  不启动 daemon、不启动 timer/reconnect loop、不做 Tauri event emission。
- placeholder 只展示 policy summary，不因为 policy 存在而自动调用任何 one-shot request，不实现真实
  retry button、loading state 或 frontend Status/History view model。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不改变 TUI/CLI 行为。

Phase 9z 增加 GUI first-screen explicit refresh affordance shape，不接真实点击：

- `src-tauri` 可以注册一个 refresh affordance shape command，固定 placeholder 阶段手动刷新控件的
  静态展示契约：label、enabled、explicit trigger、invoke target、history limit、loading=false。
- command 不创建 `DaemonClient`、不调用 `connect_default()`、不发送 IPC、不调用 one-shot summary、
  不启动 daemon、不启动 timer/reconnect loop、不做 Tauri event emission。
- placeholder 只展示 refresh affordance 静态字段，不注册真实 click handler，不自动调用任何
  one-shot request，不实现 loading state 或 frontend Status/History view model。
- 不新增 IPC protocol，不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不改变 TUI/CLI 行为。

Phase 9aa 增加 GUI first-screen explicit refresh click wiring，不自动请求：

- placeholder 可以渲染一个真实 refresh button，并且只在用户 click 后调用既有
  `gui_first_screen_summary_request_once` one-shot command。
- 初始加载仍只允许调用静态 shape/preflight commands，不得自动调用 one-shot summary，不得
  启动 daemon、service management、subscription、timer/reconnect loop 或 Tauri event emission。
- click handler 只更新 placeholder 的 loading/result/error 文本；不实现完整 Status/History
  view model，不保存 history，不改 IPC protocol，不 bump `PROTO_VERSION`。
- 不运行 Tauri dev/build/bundle，不启动 daemon/GUI，不改变 TUI/CLI 行为。

Phase 9ab 增加 GUI first-screen explicit refresh result projection，不新增请求：

- placeholder 可以在 9aa 的显式 click 成功后，把返回的 first-screen summary 投影到已有
  status/history/readiness 文本字段，验证首屏结果展示路径。
- 不新增 Tauri command，不新增 IPC command/event，不自动调用 one-shot，不订阅 daemon event
  stream，不启动 daemon、service management、timer/reconnect loop 或 Tauri event emission。
- projection 只更新当前 HTML placeholder 内的文本；不建立完整 Status/History view model，
  不保存 history，不做 metrics sink，不改变 TUI/CLI 行为。
- 不运行 Tauri dev/build/bundle，不启动 daemon/GUI。

Phase 9ac 增加 GUI first-screen explicit refresh error projection，不新增请求：

- placeholder 可以在 9aa 的显式 click 失败后，把 one-shot request error 投影到已有
  offline/action 文本字段，验证 daemon offline 或 recoverable error 展示路径。
- error projection 必须只发生在 explicit refresh click 的 catch 路径内；初始加载仍不得自动请求，
  不得实现 retry loop、service management、daemon start、subscription、timer/reconnect loop 或
  Tauri event emission。
- 不新增 Tauri command，不新增 IPC command/event，不 bump `PROTO_VERSION`，不建立完整 error
  view model，不保存 history，不改变 TUI/CLI 行为。
- 不运行 Tauri dev/build/bundle，不启动 daemon/GUI。

Phase 9ad 增加 GUI first-screen explicit refresh success clears offline display，不新增请求：

- placeholder 可以在 9ab 的显式 click 成功 projection 中清理 9ac 可能留下的 offline/error 文本，
  避免成功状态和 stale error 同屏并存。
- 清理只发生在 explicit refresh click 成功路径内；初始加载仍不得自动请求，不得新增 retry loop、
  service management、daemon start、subscription、timer/reconnect loop 或 Tauri event emission。
- 不新增 Tauri command，不新增 IPC command/event，不 bump `PROTO_VERSION`，不建立完整 view model，
  不保存 history，不改变 TUI/CLI 行为。
- 不运行 Tauri dev/build/bundle，不启动 daemon/GUI。

Phase 9ae 增加 GUI command permission 和初始化失败可见性，不新增订阅：

- `src-tauri/capabilities/default.json` 必须显式授权当前 placeholder frontend invoke 的 application
  commands，权限保持主 window 最小范围，不开启 shell/filesystem/http/process/global shortcut。
- `gui-dist/index.html` 必须在 initialization await 前绑定 refresh click handler；初始化错误必须显示在
  现有 action status/result 字段，不能静默吞掉。
- 不实现 daemon event subscription、recording state streaming、reconnect supervisor、service
  management 或自动首屏 one-shot。

Phase 9ag 增加手动 Refresh 的可读首屏摘要，不新增订阅：

- `gui-dist/index.html` 可以增加 manual summary 区域，显示最近一次显式 Refresh 的 connected/state/history/
  latest preview/timing/error 文本。
- summary projection 只发生在 explicit refresh click 的 success/catch 路径内；初始加载仍不得自动调用
  one-shot，不得订阅 daemon event stream，不得启动 reconnect loop、timer、daemon 或 service management。
- 不新增 backend command，不新增 IPC command/event，不建立完整 Status/History view model。

Phase 9ah 增加 frontend first-screen view model preflight，不新增订阅：

- `gui-dist/index.html` 可以维护本地 `firstScreenViewModel`，聚合 connected/state/history/latest/
  timing/error/last refresh status，并投影到现有 manual summary 字段。
- view model 只能由 initialization 和 explicit Refresh success/catch 更新；不得订阅 daemon event
  stream，不得调用 `Subscribe`，不得启动 reconnect loop、timer、daemon 或 service management。
- 不新增 backend command，不新增 IPC command/event，不建立完整 Status/History view model。

Phase 9ai 增加 GUI backend daemon event stream start command，不实现 reconnect：

- `src-tauri` 可以注册显式 `gui_start_daemon_event_stream` command，由 frontend 调用后启动 GUI-owned
  background task。
- task 连接现有 daemon IPC、发送既有 `Command::Subscribe`，读取 daemon events，并通过 Tauri event
  emit 给 main window。payload 只覆盖 first-screen event / connection state / recoverable problem。
- 不实现 reconnect supervisor、retry backoff、service management、daemon auto-start 或完整 window close
  cancellation；可用一次性 started 标记避免重复启动。
- 不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为。

Phase 9aj 增加 frontend daemon event listener wiring，不实现 reconnect：

- `gui-dist/index.html` 可以在初始化期间注册 `window.__TAURI__.event.listen("shuohua://daemon-event", ...)`，
  然后显式调用 `gui_start_daemon_event_stream` 启动 9ai backend bridge。
- incoming event payload 只能投影到现有 `firstScreenViewModel` 和 DOM 字段，用于显示 recording
  state、history changed 和 recoverable problem；不得新增 recording controls 或完整 Status/History view。
- 不实现 reconnect supervisor、retry timer、service management、daemon auto-start、start/stop/cancel
  recording command 或 release build/bundle。
- 不新增 backend command，不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为。

Phase 9ak 修复 GUI event stream state forwarding，不新增 IPC：

- 9ai/9aj 验证发现不点 Refresh 时 GUI 不变化，因为 backend stream 只转发了 `Snapshot` /
  `DaemonStatus` / `HistoryChanged` / `Error`，但录音开始/停止来自既有 `StateChanged` event。
- `src-tauri` 可以把既有 `Event::StateChanged` 映射为当前 frontend 已消费的 `daemonStatus`
  payload；不新增 Tauri event name、不新增 IPC event、不 bump `PROTO_VERSION`。
- stream loop 不得先用 shared first-screen classifier 过滤再调用 payload mapper，否则 `StateChanged`
  会在进入 mapper 前被丢弃。
- 不新增 recording controls、reconnect supervisor、service management、daemon auto-start 或 release
  build/bundle。

Phase 9al 增加 GUI event stream first-screen data projection，不新增 IPC：

- `src-tauri` 可以把既有订阅事件 `StatsChanged`、`Partial`、`Segment`、`HistoryAppended`
  映射到当前 `shuohua://daemon-event` payload。
- frontend 可以用这些 payload 自动更新现有 placeholder 字段，覆盖 live stats/text/latest record；
  不自动触发 Refresh，不读取文件，不建立完整 History view。
- 不新增 IPC command/event，不 bump `PROTO_VERSION`，不新增 recording controls、reconnect supervisor、
  service management、daemon auto-start 或 release build/bundle。

Phase 9al 后冻结 GUI 产品化开发：

- 当前 GUI 只作为 Tauri 独立 client、daemon IPC one-shot request、daemon event bridge、
  frontend listener 和 capability permission 的集成 PoC。
- 不继续把 `gui-dist/index.html` placeholder 打磨成最终 GUI；最终 GUI 会包含登录、历史展示、
  状态展示、复杂配置交互和 onboarding，需要单独产品设计。
- 下一主线转回 Phase 7/8/10：Windows/Linux 原生 overlay 和非 macOS TUI/overlay 可用性。
- 后续只维护 GUI 接口边界不回退，不新增 reconnect supervisor、recording controls、
  service management 或 release/bundle 指标，除非重新进入 GUI 产品阶段。

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

当前顺序：

1. 保留已完成的 Linux compile/capability/service dry-run 基线，作为 Unix 侧回归保护。
2. Windows-first core：path/config/state、IPC endpoint security、single instance runtime、audio、
   overlay、hotkey、clipboard/paste。
3. Windows artifact/CI 优先于反复让用户在 Windows 上手动构建。
4. Linux runtime backend 在 Windows 核心路径稳定后继续推进。

选择 Windows first 是因为 Windows 与 macOS/Linux 的桌面、安全、IPC、启动和输入模型差异最大。
先把 Windows 的约束落到接口和 runtime 验证里，可以降低后续从 Linux 回补 Windows 时重改边界的风险。
Linux compile/cfg 基线仍保留，用于防止共享代码破坏非 macOS 编译。

Phase 10a cross-check baseline:

- Shared network clients use target-specific TLS features. Linux uses Rustls to avoid OpenSSL sysroot
  coupling during cross-check: `reqwest` uses `rustls-tls` and `tokio-tungstenite` uses
  `rustls-tls-webpki-roots`. Non-Linux targets keep native TLS for now to avoid introducing `ring`
  MSVC sysroot requirements into the Windows check path.
- `make check-windows` runs `cargo check --target x86_64-pc-windows-msvc`.
- `make check-linux` runs `cargo check --target x86_64-unknown-linux-gnu`.
- On macOS, Windows check can validate cfg/type boundaries but still does not prove Windows runtime behavior.
- On macOS, Linux check currently needs a Linux C cross compiler/sysroot for native build scripts such as `ring`;
  Docker/cross/CI or a Linux VM should provide that environment.

Phase 10b TUI capability diagnostics:

- Add a read-only platform capability summary to the existing TUI Status page.
- The TUI must consume the same static platform/renderer capability snapshots as doctor; it must not probe
  permissions, create overlay windows, open IPC transports, or start background tasks.
- The summary should show counts for available/unsupported/unavailable/partial/degraded/unknown and list
  non-available capability details with backend/reason/next step.
- This is a diagnostics visibility step only; it does not implement Windows/Linux overlay, hotkey, clipboard,
  service, or IPC backends.

Phase 10c Docker/cross Linux check baseline:

- `make check-linux-cross` runs
  `DOCKER_DEFAULT_PLATFORM=linux/amd64 cross check --target x86_64-unknown-linux-gnu`, with Docker
  container proxy variables pointed at `host.docker.internal:7890` for the current macOS Docker Desktop
  environment.
- This is the preferred macOS-hosted Linux cfg/type check because Docker provides the Linux sysroot and C
  toolchain required by native build scripts such as `ring`.
- Apple Silicon Docker needs the explicit `linux/amd64` platform because the upstream
  `ghcr.io/cross-rs/x86_64-unknown-linux-gnu:0.2.5` image does not publish a Linux arm64 manifest.
- The upstream image currently carries `127.0.0.1:7890` proxy variables in this environment; inside Docker
  that address is the container itself. The make target overrides the outer `cross` environment to
  `host.docker.internal:7890`.
- `Cross.toml` installs `pkg-config libasound2-dev` for the Linux GNU container because `cpal`'s Linux ALSA
  backend compiles `alsa-sys` and needs `alsa.pc` from the sysroot.
- Linux should not download ONNX Runtime at build time during cross checks. The Linux target uses
  a local Silero unavailable stub and does not depend on `voice_activity_detector`, because that crate's
  current `load-dynamic` feature still leaves `ort` default features enabled and pulls
  `ort-sys/download-binaries` plus `native-tls` into the Docker build. Linux ONNX Runtime/VAD provisioning
  remains a later runtime design item.
- `cross` still requires Docker/Podman to be running and may require rustup metadata for the target-specific
  stable toolchain on macOS. If it fails before starting Docker with
  `toolchain 'stable-x86_64-unknown-linux-gnu' may not be able to run on this system`, install it with:

  ```sh
  rustup toolchain add stable-x86_64-unknown-linux-gnu --profile minimal --force-non-host
  ```

- Passing this check proves Linux compile/cfg boundaries only. It does not prove Linux service lifecycle,
  desktop permissions, hotkey capture, overlay compositor behavior, or runtime IPC behavior.

Phase 10d Linux compile-time capability sync:

- `current_platform_capabilities()` should map Linux compile-checked primitives instead of reporting every
  capability as generic `backend_not_implemented`.
- Linux `ipc.transport`, `daemon.single_instance`, and `process.probe` are `available` because they use
  existing Unix implementations and compile under Docker/cross.
- Linux `audio.capture` is `partial/cpal_alsa/compile_checked` because ALSA sysroot compilation passes but
  real device enumeration and permissions are not verified.
- Linux `service.manager` remains `unsupported/systemd_user_skeleton/backend_not_implemented`; Phase 10d
  does not implement systemd user service management.
- This is diagnostics truthfulness only. It does not start daemon on Linux, install service files, implement
  hotkey/clipboard/text injection, or validate overlay runtime behavior.

Phase 10e Linux systemd user dry-run/status skeleton:

- Add a Linux-only service manager backend that can build the systemd user unit path/body and print
  dry-run status information.
- `shuo service status` may show daemon IPC status plus `systemd.user: dry-run unit ...`; it must not call
  `systemctl --user`.
- `install`, `uninstall`, `start`, `stop`, and `restart` remain unsupported on Linux until the runtime
  behavior is validated on a Linux machine.
- This phase does not add service CLI flags, does not write unit files, and does not implement smart fallback.

Phase 10f Linux service manager capability sync:

- Update the Linux static capability snapshot so `service.manager` reflects the Phase 10e dry-run/status
  skeleton instead of generic unsupported.
- The status should be `partial`, backend `systemd_user_dry_run`, reason `dry_run_status_only`, with a next
  step that points to real systemd user install/start validation on Linux.
- This is diagnostics truthfulness only. It must not implement install/start/stop/restart, write unit files,
  call `systemctl --user`, or declare Linux service management runtime-ready.

Phase 10g Path Open/Reveal Facade:

- Move TUI config/audio open/reveal commands behind a small `platform::path` facade.
- macOS keeps current `open` / `open -R` behavior.
- Linux may use `xdg-open` for open directory/file and reveal fallback; it must be reported as
  `partial/xdg_open` because Linux file managers do not share a reliable reveal-file contract.
- Windows remains unsupported in this phase unless a separate Windows shell-open design is added.
- This phase must not change history/audio path safety checks, config editor `$VISUAL`/`$EDITOR` behavior,
  or daemon hot paths.

Phase 10h Windows Path Open/Reveal Compile Backend:

- Add a Windows `platform::path` backend that uses `explorer.exe` for open/reveal and compiles under
  `x86_64-pc-windows-msvc`.
- Mark Windows `path.open_reveal` as `partial/explorer/runtime_not_verified` because shell behavior still
  needs a Windows VM or real machine.
- This phase must not implement Windows daemon lifecycle, hotkey, clipboard, text injection, overlay runtime,
  or shell API COM integration.

Phase 10i Audio Convert Facade:

- Move retained audio conversion behind a small `platform::audio_convert` facade.
- macOS keeps the current `/usr/bin/afconvert` arguments and cleanup behavior.
- Linux/Windows return explicit unsupported for retained audio conversion until a converter backend is chosen
  and runtime-tested.
- This phase must not change retained audio path layout, history schema, recorder PCM/WAV writing, or
  `record_audio = "off"` behavior.

Phase 10j Windows Lifecycle Primitive Compile Backend:

- Replace the Windows-only `platform::lifecycle` placeholder with compile-checked Win32 primitives:
  a named mutex for daemon single-instance and `OpenProcess` for process probing.
- Mark Windows `daemon.single_instance` and `process.probe` as `partial/runtime_not_verified`.
- This phase must not implement Windows service install/start/stop, smart fallback, ACL/security descriptor
  hardening, daemon auto-start, or runtime validation claims.

Phase 10k Windows Service Manager Dry-Run Status Skeleton:

- Add a Windows-only service manager backend that can print dry-run status information for a future
  per-user service/logon task strategy.
- `install`, `uninstall`, `start`, `stop`, and `restart` remain unsupported; no registry, Task Scheduler,
  SCM, PowerShell, or file writes are allowed.
- Mark Windows `service.manager` as `partial/windows_user_dry_run/dry_run_status_only`.
- This phase must not implement smart fallback, auto-start, elevated service management, or runtime claims.

Phase 10l Non-macOS Desktop Capability Truthfulness:

- Sync Linux/Windows static capability snapshots with the existing desktop facade behavior:
  hotkey, hotkey suppression, clipboard, and text injection remain unsupported; active app is degraded
  to default/empty context; desktop permissions are unavailable because no permission probe exists.
- This phase must not implement Linux/Windows desktop APIs, hotkey capture, clipboard writes, text injection,
  permission probes, or active-window lookup.

Phase 10m Windows Development Design Baseline:

- Add `docs/cross-platform/windows.md` as the Windows-first implementation baseline.
- Record Windows per-user file layout, Named Pipe security, user-session daemon lifecycle, Task Scheduler
  startup direction, audio/hotkey/clipboard/overlay routes, artifact strategy, runtime validation order, and
  user-intervention points.
- Add `docs/cross-platform/app-data.md` as the shared CLI/daemon/GUI/packaged app data ownership model.
- This phase is docs-only. It must not change Windows behavior or promote any Windows capability.

Phase 10n Windows Runtime Validation Checklist:

- Add a Windows validation checklist document or section that the user can run directly on Windows.
- Include exact commands, expected observable behavior, and where to paste command output.
- Scope the first checklist to version/doctor/config paths, state/history/log path creation, Named Pipe daemon
  status, single-instance smoke, service dry-run status, and Explorer open/reveal.
- Do not include audio, overlay, hotkey, or paste in the first checklist until a testable artifact exists.

Phase 10o Windows Path/Config/State Backend:

- Start converging path discovery behind an `AppPaths`-style product path facade, then replace Unix-only state
  discovery in `src/paths.rs` with a Windows backend using per-user known folders: config under
  `%APPDATA%\Shuohua`, state/history/audio/logs/traces under `%LOCALAPPDATA%\Shuohua`.
- Prefer Windows known-folder APIs; allow environment fallback only when documented as development fallback.
- Add tests that protect Windows from using Unix dotfile/XDG/HOME paths.
- This phase must not change macOS path layout or config schema.

Phase 10p Windows CI Artifact Build:

- Add a `windows-latest` build path that produces a debug or release `shuo.exe` artifact suitable for manual
  smoke testing.
- Keep artifact build separate from runtime validation. CI proves build/package shape, not hotkey/audio/overlay
  behavior.
- This phase should run after the Windows path backend so first manual artifacts create data in final locations.

Phase 10q Windows Named Pipe Security And Runtime Smoke:

- Replace the skeleton pipe endpoint with a user/session scoped endpoint and explicit security descriptor/DACL.
- Preserve the JSON-line protocol and existing transport facade.
- Add compile checks/tests for endpoint naming and ACL construction where possible.
- Stop for Windows runtime testing after this phase because pipe ACL, elevation, and session behavior cannot be
  accepted from macOS.

Phase 10r Windows Desktop Runtime Sequence:

- Continue in this order after IPC runtime smoke: audio capture, overlay visible PoC, hotkey low-level hook,
  clipboard/paste.
- Each subphase should update `windows.md`, add focused tests/compile checks, implement the smallest backend,
  and stop for user Windows runtime validation before capability promotion.
- GUI product work remains frozen during these subphases.

## 持续维护

- 每完成一个 phase，更新 `overview.md` 的阶段状态。
- 发现文档假设错误，先改文档，再改实现。
- 不把 PoC 临时日志放进长期文档；只记录结论、风险和决策。
