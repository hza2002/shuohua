# GUI App

## 当前设计基线

GUI App 使用 Tauri 最新稳定版。Tauri GUI 是按需 client，不嵌入 daemon，不参与录音热路径。
daemon 未打开 GUI 时不加载 WebView。

Tauri 当前文档定位是 Rust backend + Web frontend，底层通过 WRY 使用各平台 WebView，
并提供 commands/events 与前端通信；bundler 覆盖 macOS、Windows、Linux。这个能力匹配
配置、历史、诊断、onboarding 等复杂 GUI。

如果后续 PoC 证明 Tauri 在目标平台的启动、内存、打包或系统集成成本不可接受，可以重新评估。
在有反证前，Tauri 是默认路线。

Phase 9al 后 GUI 暂停继续产品化实现。当前成果只作为未来 GUI 的接口和集成 PoC：

- `src-tauri/**` 保留为独立按需 client skeleton，证明 Tauri command、capability permission、
  daemon IPC one-shot request、daemon event bridge 和 frontend listener 可以跑通。
- `gui-dist/index.html` 仍是 placeholder/debug surface，不作为最终 GUI 信息架构或视觉设计基线。
- 后续登录、历史展示、状态展示、复杂配置交互、onboarding 和 service management 需要重新做产品
  设计，不在当前跨平台 renderer/backend 阶段继续堆半成品。
- 在 Windows/Linux overlay 和核心跨平台可用性完成前，GUI 只维护边界不回退；不继续扩展页面、
  reconnect supervisor、recording controls 或 release/bundle 指标。

## 范围

第一阶段 GUI 覆盖 TUI 的主要能力：

- Status：daemon 状态、当前 session、audio meter、ASR/post 概览。
- History：分页、搜索、详情、audio 关联、统计和图表。
- Configure：查看配置、编辑入口、模板/向导、validate/reload。
- Diagnostics：doctor、权限、服务状态、支持 bundle 入口。
- Service：install/start/stop/restart/status。

后续 GUI 可以扩展：

- 登录和账号状态。
- 更复杂的配置编辑器。
- 更强的历史浏览、筛选、导出。
- 首次启动 onboarding。

## 非目标

- GUI 不替代 daemon。
- GUI 不负责全局 hotkey、录音、ASR、post、history 落盘。
- GUI 不要求常驻；关闭 GUI 不影响 daemon。
- GUI 不作为 overlay 的默认技术方案。overlay 是否复用 Web 技术要由 overlay PoC 决定。

## 通信边界

GUI 应使用与 TUI 同级的 client API。短期可以复用 JSON-line IPC；长期可以在 daemon 侧
增加适合 GUI 的 command/event，但不得绕过 schema/history/state 的 ownership。

Tauri command 只做 GUI 进程内桥接：

```text
frontend -> Tauri command -> shuohua client API -> daemon IPC
daemon event -> shuohua client API -> Tauri event -> frontend
```

Phase 9b 先建立共享 client API 边界，不创建 Tauri workspace：

- client API 只封装 daemon 连接和既有 `ipc::protocol::Command` / `Event`，不新增 JSON-line
  wire shape，不 bump `PROTO_VERSION`。
- TUI 和后续 GUI backend 都应把它当作 daemon client 入口；GUI frontend 不能直接构造
  transport、读写 socket 或读取 history/config 文件。
- client API 可以提供 GUI 首屏需要的命令组，例如 daemon status、subscribe、history page 和
  history stats，但这些命令必须映射到现有 IPC protocol。
- client API 不依赖 Tauri、WRY、WebView、frontend build output 或 window runtime；daemon、
  TUI 和 shared client API 都不得链接 Tauri/WebView。
- Tauri command 只调用 client API，并把结果转换为前端 view model；view model 的本地化和展示
  细节留在 GUI 层，不进入 daemon protocol。

Phase 9c 扩展 GUI 首屏 helper，仍不创建 Tauri workspace：

- client API 可以暴露首屏 request helper，用于 daemon status、subscribe、history page 和
  history stats；helper 必须返回现有 `Command`，不能新增 IPC command。
- 首屏 response helper 只把现有 `Event` 分类成 GUI backend 可消费的 summary input，例如
  daemon status、snapshot、history page、history stats 和 recoverable error；不做本地化、不读
  config/history 文件、不生成前端组件 view model。
- GUI backend 后续可以把这些 summary input 转成 Tauri event 或 command response；frontend
  仍不得直接访问 IPC transport。

Phase 9d 先记录 GUI 复用边界的当前限制，不做 crate 拆分：

- 当前 crate 只有 `[[bin]] shuo`，没有 `src/lib.rs`。Phase 9b/9c 的 `client_api` 是
  binary crate 内的共享边界，能约束 TUI 和未来 GUI backend 的 API 形状，但还不能被独立
  Tauri crate 作为 Rust library 依赖。
- 真正创建 Tauri workspace 前，需要单独做 library split 评审：优先只把 `client_api`、
  `ipc::client`、`ipc::protocol`、`ipc::transport` 以及必要的数据模型移到可复用 library surface；
  不把 daemon runtime、hotkey、overlay、voice、AppKit 或 TUI 拉进 GUI backend 依赖树。
- library split 必须单独成阶段，并先用架构测试证明 daemon 热路径不依赖 GUI runtime，GUI
  backend 不依赖 daemon implementation modules。
- 在 library split 之前，不创建 Tauri app/workspace，避免 PoC 通过 path hacks、复制 IPC 类型或
  直接依赖 binary internals 形成错误基线。

Phase 9e 记录 library split audit baseline，不做 crate 拆分：

- 最小候选 library surface 仍是 `client_api`、`ipc::client`、`ipc::protocol`、
  `ipc::transport` 和必要数据模型。这个 surface 足够让后续 GUI backend 连接 daemon、发送
  首屏命令、接收和分类首屏事件。
- `ipc::protocol` 当前依赖 `history` 和 `state` 数据类型：`HistoryRecord`、
  `HistoryStatsSnapshot`、`AnalyticsSnapshot`、`AudioMeter`、`SessionMeta`、`SessionPhase`
  等。library split 不能只移动 `ipc::protocol` 文件；必须同时决定这些模型是进入 library
  surface，还是拆出更小的 wire DTO。
- `ipc::client` 依赖 `ipc::transport`；`ipc::transport` 当前是 Unix-only transport，
  直接使用 Unix domain socket 和 Unix filesystem metadata。library split 可以先保持
  macOS/Linux client 可用，但 Windows Named Pipe adapter 必须仍由后续 IPC transport backend
  阶段处理。
- 禁止把 daemon runtime、service manager、hotkey、voice、overlay、AppKit/macOS backend、TUI
  拉进 GUI library surface。GUI backend 只能依赖 daemon client API、wire protocol 和必要
  DTO。
- 在 library split 阶段之前仍不创建 Tauri workspace。先让 library boundary 编译和测试成立，
  再创建 GUI app；否则 PoC 很容易复制 IPC 类型或绕过 `client_api`。

Phase 9f 做最小 library split，仍不创建 Tauri workspace：

- 新增 `src/lib.rs`，只暴露后续 GUI backend 连接 daemon 所需的 client/protocol surface：
  `client_api`、`ipc::client`、`ipc::protocol`、`ipc::transport`，以及现有 protocol DTO 依赖的
  `history`、`state`、`paths`、`text_stats` 数据模型。
- binary 继续拥有 daemon runtime、CLI/TUI、hotkey、voice、overlay、platform backend、config
  reload 和 IPC server。library target 不暴露这些模块，也不把它们变成 GUI backend 依赖。
- `ipc::server` 暂不进入 library surface，因为它依赖 daemon runtime 的 `state`、
  `history` service、reload/config 控制面；GUI backend 只需要 client side。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 history schema，不创建
  Tauri workspace，不新增 Tauri/WRY/WebView runtime 依赖。
- `ipc::transport` 仍是 Unix-only；因此 Phase 9f 的 library client 只承诺 macOS/Linux 当前
  transport 可编译。Windows Named Pipe 仍由后续 IPC transport backend 阶段处理。

Phase 9g 增加 GUI client 连接状态骨架，仍不创建 Tauri workspace：

- shared `client_api` 可以公开 GUI backend 可复用的 daemon connection state、recoverable
  problem kind 和 retry delay helper，用于表达 daemon offline、event stream closed、read
  failure 和 reconnecting 状态。
- 这些类型只描述 client 侧状态和退避策略，不新增 daemon command/event，不 bump
  `PROTO_VERSION`，不改变 JSON-line IPC。
- 这个阶段不实现后台 reconnect task，不改变 TUI 连接行为，不自动启动 daemon，不读写配置或
  history 文件。Tauri command/event 层后续可以把这些状态转换成 frontend view model。
- retry delay 必须是有上限的短序列，避免 GUI daemon-offline 首屏 busy-loop；实际 timer 和
  cancellation ownership 留给 GUI backend。

Phase 9h 增加 GUI backend event bridge 骨架，仍不创建 Tauri workspace：

- shared `client_api` 可以公开 GUI backend event 类型，把既有 daemon `Event`、9g 的 connection
  state 和 recoverable connection problem 统一成 GUI backend 可转发的事件形状。
- bridge 只做引用级分类和封装，不 clone 大型 history payload，不做本地化，不生成 frontend
  view model，不调用 Tauri event API。
- daemon event 分类必须继续复用 Phase 9c 的 `FirstScreenEvent`，避免 GUI backend 绕过既有首屏
  helper 自己解释 IPC event。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不创建
  Tauri workspace。

Phase 9i 增加 GUI 首屏 metrics/timing 纯模型，仍不创建 Tauri workspace：

- shared `client_api` 可以公开首屏 timing/readiness 类型，用于后续 GUI backend 记录从 GUI
  启动、daemon connect、首个 daemon event 到首屏数据 ready 的耗时。
- 时间戳由 GUI backend 后续传入；shared client API 只做纯计算、饱和差值和既有
  `FirstScreenEvent` 的 readiness 判定，不调用 `Instant::now()`、timer、IPC、Tauri event API
  或 metrics sink。
- 首屏 ready 的最小自动判定只要求 daemon status、history page 和 history stats 都到达；
  snapshot、history changed 和 recoverable error 可以被记录为输入，但不能单独让首屏 ready。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不创建
  Tauri workspace。

Phase 9j 记录 Tauri permissions/capabilities preflight，仍不创建 Tauri workspace：

- Tauri v2 capabilities 是把 permissions 授权给指定 windows/webviews 的边界；PoC 只能给主
  window/webview 绑定最小 capability，不给隐藏窗口、未来 onboarding window 或任意 webview
  预授权。
- permissions 是前端可访问 command/plugin 的显式权限描述，可以包含 allow/deny 和 scopes。
  GUI PoC 的第一版只暴露 shuohua GUI backend 自有 command：首屏 snapshot、connect/reconnect
  状态、metrics readout。command 再调用 shared `client_api`；frontend 不直接访问 IPC
  transport、history/config 文件或 daemon implementation。
- PoC 不默认启用 shell、filesystem、http、process、global shortcut、updater、sidecar 管理等
  宽权限。需要打开配置目录或外部链接时，必须先单独评审 scope，并优先通过既有 CLI/client
  语义承载。
- `core:default` 可以降低样板，但 PoC preflight 不把它当作默认授权策略；创建 workspace 时应
  先列出实际需要的 core/plugin permission，再决定是否使用 default group。
- 这个阶段只记录权限边界和验收，不新增 `src-tauri/**`、不新增 Tauri/WRY/WebView 依赖、不生成
  frontend view model。

Phase 9k 记录最小 Tauri workspace 创建前验收清单，仍不创建 Tauri workspace：

- 创建 workspace 的下一阶段必须是可回滚的小步，只允许新增最小 GUI app 骨架、主
  window/webview、最小 capabilities 文件和调用 shared `client_api` 的 backend shell；不得同时
  实现完整页面、onboarding、配置编辑器或 service management。
- 允许出现的新增路径必须在实现阶段前列清：`src-tauri/tauri.conf.json`、
  `src-tauri/Cargo.toml`、`src-tauri/capabilities/*.json`、最小 frontend 入口和 GUI backend
  glue。任何 sidecar、installer asset、plugin 宽权限、复制 IPC 类型或 daemon runtime 依赖都应
  被视为 scope creep。
- 首个 workspace commit 的自动验收必须确认：daemon/CLI/TUI 不依赖 Tauri/WRY/WebView；
  `PROTO_VERSION` 不变；GUI backend 只通过 `shuohua::client_api` 和 existing IPC client
  surface 通信；TUI fallback 继续可用。
- PoC 指标必须基于 release build 记录。Tauri v2 `tauri build` 会执行 release build 并生成
  bundles/installers，使用 `tauri.conf.json` 的 `build.frontendDist` 和 build hooks；
  `tauri bundle` 面向已构建 app 生成 bundle。指标清单至少包含 bundle path/type、unsigned 或
  signed 状态、cold start、首屏 ready、open GUI idle RSS/CPU、关闭 GUI 后 daemon 存活。
  进程边界必须单独记录：daemon 未打开 GUI 时无 WebView/Tauri 进程。
- 这个阶段只记录验收清单，不新增 `src-tauri/**`、不新增 Tauri/WRY/WebView 依赖、不运行
  `tauri build` 或 `tauri bundle`。

Phase 9l 记录 GUI daemon offline/reconnect 后台任务 ownership，仍不创建 Tauri workspace：

- 后续 GUI backend 可以拥有一个 connection supervisor task，负责首次连接 daemon、发送
  `first_screen_commands()`、订阅 daemon event、应用 `reconnecting_state()` 退避并通过
  `GuiBackendEvent` 向 frontend 转发状态。这个 task 属于 GUI 进程，不能进入 daemon、TUI 或
  shared `client_api`。
- supervisor 的取消 owner 是 GUI window/app lifecycle。关闭主 window、退出 app 或切换到新的
  connection session 时必须取消旧 task；旧 task 的 late event 必须被 session id/generation
  丢弃，不能覆盖新连接状态。
- reconnect 只处理 recoverable client-side 问题：connect failed、event stream closed、read failed。
  它不能自动启动 daemon、不能安装或重启 service、不能修改配置，也不能把 daemon
  offline 当成 fatal app error。
- reconnect loop 的 timer、spawn、channel 和 Tauri event emission 只允许出现在后续 GUI backend
  crate/模块。shared `client_api` 继续只提供纯状态、退避、event bridge 和首屏 readiness/timing
  helper。
- 首屏 metrics ownership：GUI backend 记录 `gui_started_ms`、`daemon_connected_ms`、
  `first_daemon_event_ms` 和 `first_screen_ready_ms`，再调用 `FirstScreenTiming::from_marks()`；
  metrics sink 和 frontend 展示都不进入 daemon protocol。
- 这个阶段只记录 ownership/cancellation 语义，不新增 runtime loop、不新增 IPC command/event、
  不创建 `src-tauri/**`、不新增 Tauri/WRY/WebView 依赖。

Phase 9m 创建最小 Tauri workspace skeleton，不接 daemon、不实现页面：

- 只新增 Tauri 标准最小骨架文件：`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、
  `src-tauri/build.rs`、`src-tauri/src/main.rs`、`src-tauri/src/lib.rs` 和
  `src-tauri/capabilities/default.json`。不新增 frontend build 产物、installer asset、sidecar、
  plugin 宽权限或完整页面。
- skeleton 的 Rust backend 可以声明对根 crate `shuohua` 的 path dependency，后续通过
  `shuohua::client_api` 接 daemon；本阶段不得调用 IPC、启动 reconnect supervisor、启动 daemon
  或读取配置/history 文件。
- Tauri/WRY/WebView runtime 只能出现在 `src-tauri/**`。根 `Cargo.toml`、daemon、TUI 和
  shared `client_api` 仍不得出现 Tauri/WRY/WebView token，daemon 未打开 GUI 时仍不加载 WebView。
- `tauri.conf.json` 只定义主 window/webview、基础 build frontendDist 和最小 product/identifier；
  capabilities 只绑定主 window，权限保持最小，不默认开启 shell/filesystem/http/process/global
  shortcut/updater/sidecar。
- 这个阶段不运行 `tauri dev`、`tauri build` 或 `tauri bundle`，不生成 release 指标，不新增 IPC
  command/event，不改变 TUI/CLI 行为。

Phase 9n 增加最小 GUI backend shell 和静态 frontend placeholder，不接 daemon：

- `src-tauri` 可以注册一个 Tauri command，用于验证 GUI backend command wiring。command 只返回
  静态 shell metadata，例如 app 名称、当前 phase、是否存在 daemon connection 和 frontend
  placeholder readiness；不得调用 `shuohua::client_api`、`ipc::client`、`tokio::spawn`、
  timer、channel 或 metrics sink。
- frontend 只允许新增 `gui-dist/index.html` 和必要的同目录静态资源。placeholder 可以调用本地
  metadata command 并渲染返回值，但不得实现 Status/History/Diagnostics view model，不读
  config/history 文件，不直接访问 IPC transport。
- Tauri command/event API 仍只能出现在 `src-tauri/**` 和 `gui-dist/**`。daemon、TUI、root
  `Cargo.toml` 和 shared `client_api` 继续不含 Tauri/WRY/WebView runtime token。
- capabilities 仍保持最小；本阶段不新增 shell、filesystem、http、process、global shortcut、
  updater、sidecar 或 service management 权限。
- 这个阶段不运行 `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI，不新增 IPC
  command/event，不实现 reconnect supervisor。

Phase 9o 增加 GUI first-screen request plan command，不发送 IPC：

- `src-tauri` 可以注册一个 Tauri command，用于返回首屏请求计划。该 command 必须复用
  `shuohua::client_api::first_screen_commands()`，把既有 daemon `Command` 映射成 GUI 可展示的
  summary，例如 `subscribe`、`daemonStatus`、`historyPage`、`historyStats`。
- request plan 只描述“后续真实连接 daemon 时要发送什么”，可以包含 history limit、
  requires daemon connection 和 transport not opened 等静态字段；不得创建 `DaemonClient`、
  不得调用 `connect_default()`、不发送 IPC、不订阅 daemon event stream、不启动 reconnect loop。
- frontend placeholder 可以调用该 command 并展示 command count/kinds，仍不得生成真实
  Status/History/Diagnostics view model，不读 config/history 文件，不直接访问 IPC transport。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9p 增加 GUI daemon status snapshot shape command，不发送 IPC：

- `src-tauri` 可以注册一个 daemon status snapshot command，但当前只固定 GUI backend 到 frontend
  的 response shape。它描述 GUI backend 当前没有 daemon 连接、没有打开 transport、没有真实
  status event，并标记后续真实快照需要发送既有 `Command::DaemonStatus`。
- 该 command 是“shape preflight”，不是 status client：不得创建 `DaemonClient`、不得调用
  `connect_default()`、不得调用 `send_command`、不得订阅 daemon event stream、不得启动
  reconnect loop 或 timer。
- 返回字段保持前端可直接展示但不承担本地化：`connected`、`transportOpened`、
  `snapshotAvailable`、`requestKind` 和 `stateLabel`。真实 `Event::DaemonStatus` 到 view model 的
  映射留给后续连接阶段。
- frontend placeholder 可以展示这个静态 status snapshot shape；仍不得实现真实
  Status/History/Diagnostics view model，不读取 config/history 文件，不直接访问 IPC transport。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9q 增加 GUI daemon status event mapper，不发送 IPC：

- `src-tauri` 可以增加一个纯 mapper，把调用方已经拿到的既有 `Event::DaemonStatus` 转成
  Phase 9p 固定的 daemon status snapshot response shape。mapper 只处理 status event，不解释
  snapshot、history、error 或 config reload event。
- mapper 输出字段保持稳定：`connected=true`、`transportOpened=true`、
  `snapshotAvailable=true`、`stateLabel`、`pid`、`uptimeMs`、`recordingId`，并继续带
  `request.requestKind=daemonStatus`。空 snapshot helper 继续表达未连接状态。
- 该阶段仍不是真实 status client：不得创建 `DaemonClient`、不得调用 `connect_default()`、
  不得调用 `send_command`、不得订阅 daemon event stream、不得启动 reconnect loop 或 timer。
- `gui_daemon_status_snapshot` command 继续返回未连接静态 shape。真实连接、发送
  `Command::DaemonStatus`、读取 `Event::DaemonStatus` 和 Tauri event emission 留给后续阶段。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9r 增加 GUI daemon status one-shot request command，不订阅、不重连：

- `src-tauri` 可以注册一个 `gui_daemon_status_request_once` command。它只在前端显式调用时
  连接现有 daemon IPC，发送既有 `Command::DaemonStatus`，等待一个既有
  `Event::DaemonStatus`，然后复用 9q 的 mapper 返回 status snapshot shape。
- command 不进入 daemon、TUI 或 shared `client_api`；Tauri/WebView runtime 仍只在
  `src-tauri/**`。placeholder 当前不自动调用这个 command，避免打开静态页面时默认连接 daemon。
- 错误返回独立 request error shape：`kind`、`message`、`recoverable`。connect failed、
  IPC write failed、IPC read failed、daemon closed 和 daemon `Event::Error` 都是 recoverable
  GUI request failure；它们不能自动启动 daemon、安装 service 或修改配置。
- 该阶段不订阅 daemon event stream，不调用 `Subscribe`，不启动 reconnect loop/timer，不做
  Tauri event emission，不实现真实 frontend Status/History/Diagnostics view model。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9s 增加 GUI history summary one-shot request command，不订阅、不重连：

- `src-tauri` 可以注册一个 `gui_history_summary_request_once` command。它只在前端显式调用时
  连接现有 daemon IPC，发送既有 `Command::GetHistory { limit, before: None,
  before_id: None, query: None }` 和 `Command::GetHistoryStats`，等待既有 `Event::History` 与
  `Event::HistoryStats`，然后返回小型 history summary shape。
- summary 只固定 GUI backend 到 frontend 的最小首屏数据：transport/summary 可用性、limit、
  page record count、matched、page aggregate stats、stats status、total/current month/today
  aggregate stats、latest record id/status/text preview。它不是完整 History view model，不包含
  详情页、搜索状态、分页 cursor、audio asset、图表或本地化文案。
- command 不进入 daemon、TUI 或 shared `client_api`；Tauri/WebView runtime 仍只在
  `src-tauri/**`。placeholder 当前不自动调用这个 command，避免打开静态页面时默认连接 daemon。
- 错误返回独立 request error shape：`kind`、`message`、`recoverable`。connect failed、
  IPC write failed、IPC read failed、daemon closed 和 daemon `Event::Error` 都是 recoverable
  GUI request failure；它们不能自动启动 daemon、安装 service、修改配置或读取 history 文件。
- 该阶段不订阅 daemon event stream，不调用 `Subscribe`，不启动 reconnect loop/timer，不做
  Tauri event emission，不实现真实 frontend History/Diagnostics view model。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9t 增加 GUI first-screen summary one-shot request command，不订阅、不重连：

- `src-tauri` 可以注册一个 `gui_first_screen_summary_request_once` command。它只在前端显式调用时
  打开一次现有 daemon IPC 连接，发送既有 `Command::DaemonStatus`、
  `Command::GetHistory { limit, before: None, before_id: None, query: None }` 和
  `Command::GetHistoryStats`，等待既有 `Event::DaemonStatus`、`Event::History` 与
  `Event::HistoryStats`，然后返回组合首屏 summary shape。
- summary 只组合 Phase 9r 的 status snapshot shape 和 Phase 9s 的 history summary shape，并带
  `historyLimit`、`transportOpened`、`summaryAvailable` 和 request metadata。它不是前端首屏
  view model，不负责布局、本地化、loading state、retry 按钮、metrics 展示或 event stream。
- command 不进入 daemon、TUI 或 shared `client_api`；Tauri/WebView runtime 仍只在
  `src-tauri/**`。placeholder 当前不自动调用这个 command，避免打开静态页面时默认连接 daemon。
- 错误返回独立 request error shape：`kind`、`message`、`recoverable`。connect failed、
  IPC write failed、IPC read failed、daemon closed 和 daemon `Event::Error` 都是 recoverable
  GUI request failure；它们不能自动启动 daemon、安装 service、修改配置或读取 history 文件。
- 该阶段不订阅 daemon event stream，不调用 `Subscribe`，不启动 reconnect loop/timer，不做
  Tauri event emission，不实现真实 frontend Status/History/Diagnostics view model。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9u 增加 GUI first-screen summary request timing，不订阅、不重连：

- `gui_first_screen_summary_request_once` 可以在 GUI backend 本地记录本次显式 request 的 timing：
  `connectDurationMs`、`firstEventMs`、`readyMs` 和 `requestDurationMs`。这些字段只描述一次
  foreground command invocation，不写入 daemon protocol、history、trace 或 shared `client_api`。
- timing 由 `src-tauri` command 使用 `std::time::Instant` 计算；允许记录 request start、
  connect completed、first matched daemon event 和 summary ready 的 elapsed milliseconds。
  不使用 `tokio::time`、不启动 timer task、不创建 metrics sink、不做 Tauri event emission。
- timing 只附着在 9t 的 first-screen summary shape 上；9r status one-shot 和 9s history summary
  one-shot 的 response shape 暂不扩展，避免多个 command 同时改动。
- 该阶段不订阅 daemon event stream，不调用 `Subscribe`，不启动 reconnect loop/timer，不做
  frontend loading/retry UI，不实现真实 Status/History/Diagnostics view model。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9v 增加 GUI first-screen explicit refresh shape，不自动请求：

- `src-tauri` 可以注册一个 `gui_first_screen_refresh_shape` command，用于固定后续前端手动刷新入口
  的 response shape。shape 只描述刷新必须由用户显式触发、默认 history limit、是否需要 daemon
  连接、当前未打开 transport，以及真实执行时应调用既有 `gui_first_screen_summary_request_once`。
- 该 command 是 refresh preflight，不是真实 refresh client：不得创建 `DaemonClient`、不得调用
  `connect_default()`、不得发送 IPC、不得调用 `gui_first_screen_summary_request_once`、不得订阅 daemon
  event stream、不得启动 reconnect loop 或 timer。
- placeholder 可以展示 refresh shape 的静态字段，但不得自动调用
  `gui_first_screen_summary_request_once`，不得实现 loading/retry UI、Status/History view model、
  service management 或配置编辑器。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9w 增加 GUI first-screen readiness/timing display shape，不自动请求：

- `src-tauri` 可以注册一个 `gui_first_screen_readiness_shape` command，用于固定后续首屏
  readiness/timing 展示的空态 response shape。shape 只描述当前 placeholder 尚未连接 daemon、
  `ready=false`、daemon status/history page/history stats 都未到达，以及 timing 字段暂不可用。
- 该 command 是 display preflight，不是真实 readiness tracker：不得创建 `DaemonClient`、不得调用
  `connect_default()`、不得发送 IPC、不得调用 `gui_first_screen_summary_request_once`、不得调用
  `std::time::Instant::now()`、不得启动 timer/reconnect loop 或 Tauri event emission。
- placeholder 可以展示 readiness/timing 空态字段，但不得实现 loading/retry UI、真实 Status/History
  view model、metrics sink、service management 或配置编辑器。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9x 增加 GUI first-screen offline/error display shape，不自动恢复：

- `src-tauri` 可以注册一个 `gui_first_screen_offline_shape` command，用于固定后续首屏 daemon
  offline / recoverable request error 的静态展示 shape。shape 只描述当前未连接 daemon、问题类型、
  是否 recoverable、是否允许 retry，以及不允许自动启动 daemon 或 service management。
- 该 command 是 offline/error display preflight，不是真实 reconnect supervisor：不得创建
  `DaemonClient`、不得调用 `connect_default()`、不得发送 IPC、不得调用
  `gui_first_screen_summary_request_once`、不得启动 daemon、不得安装/重启 service、不得启动
  timer/reconnect loop 或 Tauri event emission。
- placeholder 可以展示 offline/error 静态字段，但不得实现真实 retry button 行为、loading state、
  Status/History view model、metrics sink、service management 或配置编辑器。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9y 增加 GUI first-screen command invocation policy shape，不自动请求：

- `src-tauri` 可以注册一个 `gui_first_screen_command_policy_shape` command，用于固定 placeholder
  阶段哪些 GUI backend command 允许自动调用、哪些 command 必须由用户显式触发。自动调用只允许
  本地静态 shape/preflight commands；所有会打开 daemon transport 的 one-shot request 都必须保持
  explicit-only。
- 该 command 是 policy preflight，不是真实 command dispatcher：不得创建 `DaemonClient`、不得调用
  `connect_default()`、不得发送 IPC、不得调用 `gui_first_screen_summary_request_once`、不得启动
  daemon、不得启动 timer/reconnect loop 或 Tauri event emission。
- placeholder 可以展示 policy summary，但不得因为 policy 存在而自动调用任何 one-shot request、
  实现真实 retry button 行为、loading state、Status/History view model、metrics sink、
  service management 或配置编辑器。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9z 增加 GUI first-screen explicit refresh affordance shape，不接真实点击：

- `src-tauri` 可以注册一个 `gui_first_screen_refresh_affordance_shape` command，用于固定 placeholder
  阶段“手动刷新”控件的静态展示和安全约束。shape 可以包含 label、enabled、explicit trigger、
  invoke target、history limit、loading=false、source 等字段，但不代表真实按钮已经接线。
- 该 command 是 affordance preflight，不是真实 click handler：不得创建 `DaemonClient`、不得调用
  `connect_default()`、不得发送 IPC、不得调用 `gui_first_screen_summary_request_once`、不得启动
  daemon、不得启动 timer/reconnect loop 或 Tauri event emission。
- placeholder 可以展示 refresh affordance summary，但不得注册真实 click handler、不得因为
  affordance 存在而自动调用任何 one-shot request、不得实现 loading state、Status/History
  view model、metrics sink、service management 或配置编辑器。
- 这个阶段不新增 IPC command/event，不 bump `PROTO_VERSION`，不改变 TUI/CLI 行为，不运行
  `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9aa 增加 GUI first-screen explicit refresh click wiring，不自动请求：

- placeholder 可以把 9z 的 refresh affordance 渲染成真实 button，并注册一个 click handler。
  click handler 只在用户显式点击后调用既有 `gui_first_screen_summary_request_once` one-shot
  command；初始加载仍不得自动调用该 one-shot command。
- click handler 可以在 placeholder 内更新 loading/result/error 文本，用于验证显式请求路径。
  这个阶段不建立完整 Status/History view model，不写 history，不做 metrics sink，不做 service
  management，不自动启动 daemon。
- 该阶段不得新增 IPC command/event，不得 bump `PROTO_VERSION`，不得订阅 daemon event stream，
  不得启动 timer/reconnect loop 或 Tauri event emission；`src-tauri` 仍只复用既有显式 one-shot
  command。
- 这个阶段不运行 `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI，不改变
  TUI/CLI 行为。

Phase 9ab 增加 GUI first-screen explicit refresh result projection，不新增请求：

- placeholder 可以在 9aa 的显式 click 成功后，把 `gui_first_screen_summary_request_once` 返回的
  summary 投影到已有 status/history/readiness 文本字段：例如 connected、status state、
  history record count、readiness ready 和 action result。
- projection 必须只发生在 explicit refresh click 成功路径内；初始加载仍不得自动调用 one-shot，
  不得引入 daemon event subscription、reconnect loop、timer、Tauri event emission 或 service
  management。
- 这个阶段不新增 Tauri command，不新增 IPC command/event，不 bump `PROTO_VERSION`，不建立完整
  Status/History view model，不写 history，不做 metrics sink，不改变 TUI/CLI 行为。
- 这个阶段不运行 `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9ac 增加 GUI first-screen explicit refresh error projection，不新增请求：

- placeholder 可以在 9aa 的显式 click 失败后，把 one-shot request error 投影到已有
  offline/action 文本字段：例如 action status、action result、offline problem、recoverable 和
  retry allowed。
- error projection 必须只发生在 explicit refresh click 的 catch 路径内；初始加载仍不得自动调用
  one-shot，不得实现 retry loop、service management、daemon start、daemon event subscription、
  reconnect loop、timer 或 Tauri event emission。
- 这个阶段不新增 Tauri command，不新增 IPC command/event，不 bump `PROTO_VERSION`，不建立完整
  error view model，不写 history，不做 metrics sink，不改变 TUI/CLI 行为。
- 这个阶段不运行 `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9ad 增加 GUI first-screen explicit refresh success clears offline display，不新增请求：

- placeholder 可以在 9ab 的 explicit refresh success projection 中清理 9ac 可能留下的
  offline/error 文本字段，例如 offline problem、recoverable 和 retry allowed，避免成功结果和
  stale error 同屏并存。
- 清理必须只发生在 explicit refresh click 成功路径内；初始加载仍不得自动调用 one-shot，不得新增
  retry loop、service management、daemon start、daemon event subscription、reconnect loop、
  timer 或 Tauri event emission。
- 这个阶段不新增 Tauri command，不新增 IPC command/event，不 bump `PROTO_VERSION`，不建立完整
  Status/History/error view model，不写 history，不做 metrics sink，不改变 TUI/CLI 行为。
- 这个阶段不运行 `tauri dev`、`tauri build` 或 `tauri bundle`，不启动 daemon/GUI。

Phase 9ae 增加 GUI command permission 和初始化失败可见性，不新增订阅：

- Tauri v2 capability 必须显式授权 placeholder 前端实际调用的 application command。应用内
  command permission 使用不带 plugin 前缀的 `allow-<command-name-kebab-case>` 名称；主 window
  只授权当前首屏需要的 command，不启用 shell/filesystem/http/process/global shortcut 等宽权限。
- placeholder 的 refresh click handler 必须在任何 awaited initialization invoke 之前绑定。如果
  初始化阶段某个静态 command 被 ACL 或 backend error 拒绝，页面必须把错误投影到现有
  `refresh-action-status` / `refresh-action-result`，不能静默吞掉后让 Refresh 看起来无反应。
- 本阶段仍不实现 daemon event subscription、recording state streaming、reconnect supervisor、
  service management 或自动首屏 one-shot。用户录音时 GUI 不自动变化是当前 placeholder 的已知缺口。

Phase 9af 启用 Tauri global API 并显式显示 missing API，不新增订阅：

- 当前 frontend 是无 bundler 的静态 HTML，直接使用 `window.__TAURI__.core.invoke`。Tauri v2 只有在
  `app.withGlobalTauri = true` 时才会向 `window.__TAURI__` 注入 API；否则 initialization 和
  Refresh click 都必须显示 `tauri-api-missing`，不得静默 return。
- `withGlobalTauri` 只用于当前静态 placeholder。后续若引入 frontend package/bundler，可以改为
  `@tauri-apps/api/core` import，并同步移除 global API 依赖和对应测试。
- 本阶段仍不实现 daemon event subscription、recording state streaming、reconnect supervisor、
  service management 或自动首屏 one-shot。

Phase 9ag 增加手动 Refresh 的可读首屏摘要，不新增订阅：

- placeholder 可以增加一个简洁的 manual summary 区域，用于显示最近一次显式 Refresh 的结果：
  daemon 连接状态、daemon state、history record count、latest record preview、request timing 和错误摘要。
- summary projection 只发生在 explicit refresh click 的 success/catch 路径内。初始加载仍不得自动调用
  `gui_first_screen_summary_request_once`，不得订阅 daemon event stream，不得启动 reconnect loop、
  timer、daemon 或 service management。
- 本阶段不新增 backend command、不新增 IPC command/event、不建立完整 Status/History view model。
  它只是让下一次用户手动验证能看懂 Refresh 是否成功。

Phase 9ah 增加 frontend first-screen view model preflight，不新增订阅：

- placeholder 可以在静态 HTML 内维护一个小型 `firstScreenViewModel`，聚合当前页面已展示的
  connected、state、history count、latest preview、timing、error、last refresh status。
- 当前阶段 view model 只能由 initialization 和 explicit Refresh success/catch 更新，再投影到现有
  DOM 字段；不得连接 daemon event stream、不得调用 `Subscribe`、不得启动 reconnect loop 或 timer。
- 这个 view model 是后续 Tauri event subscription 的前端落点预演，不是完整 Status/History
  view model；不新增 backend command、不新增 IPC command/event。

Phase 9ai 增加 GUI backend daemon event stream start command，不实现 reconnect：

- `src-tauri` 可以注册一个显式 `gui_start_daemon_event_stream` command。该 command 只由 frontend
  调用后启动 GUI-owned background task：连接现有 daemon IPC、发送既有 `Command::Subscribe`，
  读取 daemon events，并把可识别的 first-screen event 转成 Tauri event 发给 main window。
- event stream task 只能属于 GUI 进程；不得进入 daemon、TUI、root runtime 或 shared `client_api`。
  shared `client_api` 仍只提供纯 event classifier/bridge 类型。
- 本阶段不实现 reconnect supervisor、retry backoff、service management、daemon auto-start 或
  自动多 session cancellation。command 可以用一次性 started 标记避免重复启动；窗口关闭后的完整
  cancellation 属后续阶段。
- 不新增 IPC command/event，不 bump `PROTO_VERSION`。Tauri event name 和 payload 属 GUI 进程内桥接。

Phase 9aj 增加 frontend daemon event listener wiring，不实现 reconnect：

- placeholder 可以在 initialization 期间通过 `window.__TAURI__.event.listen("shuohua://daemon-event", ...)`
  注册 Tauri event listener，再显式调用 `gui_start_daemon_event_stream` 启动 9ai backend bridge。
- listener 只把 incoming payload 投影到现有 `firstScreenViewModel` 和 DOM 字段。`snapshot` /
  `daemonStatus` 更新 connection/state，`historyChanged` 标记 history stale，`daemonError` /
  `connectionProblem` 更新 recoverable error display。
- 本阶段不实现 reconnect supervisor、retry timer、service management、daemon auto-start、window close
  cancellation 或 recording controls。没有 daemon 时可以显示 recoverable problem，但不得尝试启动服务。
- 不新增 backend command、不新增 IPC command/event、不 bump `PROTO_VERSION`、不建立完整
  Status/History view model。

Phase 9ak 修复 GUI event stream state forwarding，不新增 IPC：

- GUI backend event stream 必须把既有 daemon `StateChanged` event 映射成 `daemonStatus`
  payload，否则录音开始/停止只更新 TUI/IPC subscriber，GUI listener 不会自动变化。
- stream loop 不得先用 shared `gui_backend_event_from_daemon_event()` 过滤再调用 payload mapper，
  因为该 shared first-screen classifier 不包含 `StateChanged`。
- 该修复只扩展 9ai 的 payload mapper；不新增 IPC event、不 bump `PROTO_VERSION`、不新增 recording
  control command，不改变 daemon/TUI 行为。
- frontend 继续消费同一个 `shuohua://daemon-event` 和既有 `daemonStatus` payload。

Phase 9al 增加 GUI event stream first-screen data projection，不新增 IPC：

- GUI backend event stream 可以把既有 `StatsChanged`、`Partial`、`Segment` 和 `HistoryAppended`
  映射到同一个 `shuohua://daemon-event` payload。
- frontend 只更新现有 first-screen placeholder 字段：manual history count、latest preview、
  request/status summary 和 live text。Refresh 仍作为 one-shot 对照，不被自动触发。
- 不新增 IPC event、不 bump `PROTO_VERSION`、不轮询、不新增 reconnect supervisor、service
  management 或 recording controls。

## 验收指标

GUI PoC 进入实现前建议记录：

- macOS/Windows/Linux 打包体积。
- 冷启动时间。
- 打开 GUI 后空闲内存。
- 空闲 CPU。
- 与 daemon 连接断开/重连行为。

这些指标用于决定 GUI 是否可选打包、是否默认安装，以及是否需要 lazy-load 大型页面。

## Phase 9a Tauri GUI PoC Baseline

Phase 9a 先记录 Tauri GUI PoC 基线，不写 GUI app，不把 WebView 放进 daemon。
当前依据 Tauri v2 官方文档的判断：

- Tauri 仍应作为独立按需 client。`shuo daemon` 不链接 Tauri runtime、不创建 WebView、不运行
  frontend build step；GUI 关闭后 daemon 继续独立运行。
- Tauri command/event 只做 GUI 进程内桥接。前端通过 Tauri command 调用 GUI backend，
  GUI backend 再使用 shuohua client API 连接 daemon IPC；daemon event 通过 GUI backend
  转成 Tauri event 给前端。
- Tauri v2 permissions/capabilities 必须纳入 PoC。只给主 window 授权需要的 command/plugin，
  不打开 shell/filesystem/http 等宽权限；如需外部打开配置目录，优先复用已有 client API 或
  明确 scope。
- sidecar/external binary 不是默认路线。Tauri 文档支持 `bundle.externalBin` 和 shell plugin
  管理 sidecar，但 shuohua 的 daemon 已有 service lifecycle；GUI PoC 不应把 daemon 常驻进程
  变成 GUI 子进程。只有安装包分发需要同捆二进制时，才评审 sidecar。
- `tauri build`/`tauri bundle` 支持 release build、bundle 选择、target triple 和平台配置合并；
  PoC 指标应基于 release build 记录，而不是 dev server。
- 前端第一屏只做实际 client 功能：Status snapshot、History summary、Diagnostics summary。
  不做营销首页，不做配置编辑器完整版。

Phase 9 PoC checklist：

- 进程边界：daemon 未启动 GUI 时无 WebView/Tauri 进程；GUI 退出不停止 daemon。
- IPC：GUI 能连接现有 daemon transport，展示 status snapshot 和 history summary；daemon
  不在线时显示可恢复错误并支持重连。
- 安全：列出 Tauri capabilities/permissions 文件，确认未启用无关 shell/filesystem/http 权限。
- 指标：macOS/Windows/Linux release bundle size、cold start time、open GUI idle RSS、
  open GUI idle CPU、连接 daemon 首次数据时间。
- 打包：记录 `tauri build` 和 `tauri bundle` 产物路径、bundle 类型和签名/未签名状态。
- 回退：TUI 继续可用；GUI PoC 不改变 CLI/TUI 命令和 JSON-line IPC protocol。

Phase 9b 验收：

- 存在共享 daemon client API 边界，TUI 至少通过该边界获取 `IpcClient` 类型。
- `Cargo.toml` 不新增 Tauri/WRY/WebView 依赖。
- `src/daemon/**`、`src/tui/**` 和共享 client API 不出现 Tauri/WRY/WebView token。
- IPC protocol round-trip 测试继续通过，`PROTO_VERSION` 不变。

Phase 9c 验收：

- client API 提供首屏 request helper，覆盖 subscribe、daemon status、history page、
  history stats。
- request helper 和 response classifier 都只使用既有 `ipc::protocol::Command` / `Event`。
- `PROTO_VERSION` 仍为 2；不新增 Tauri/WRY/WebView 依赖，不启动 daemon/GUI。

Phase 9d 验收：

- 文档明确当前 `client_api` 仍在 binary crate 内，尚不是外部 GUI crate 可依赖的 library API。
- 架构测试保护当前状态：没有 `src/lib.rs`、没有 Tauri workspace、没有 GUI runtime 依赖。
- 后续 library split 的最小候选 surface 和禁止依赖方向有明确记录。

Phase 9e 验收：

- 记录 library split 的最小 surface：`client_api`、`ipc::client`、`ipc::protocol`、
  `ipc::transport` 和必要 DTO。
- 记录阻塞点：`ipc::protocol` 依赖 `history` / `state` 模型，`ipc::transport` 当前是
  Unix-only transport。
- 明确 library split 前仍不创建 Tauri workspace，不把 daemon runtime 或平台 UI backend
  暴露给 GUI backend。

Phase 9f 验收：

- `src/lib.rs` 存在，并只公开最小 client/protocol surface 和必要 DTO 模块。
- 外部 crate 可通过 `shuohua::client_api`、`shuohua::ipc::client`、
  `shuohua::ipc::protocol` 和 `shuohua::ipc::transport` 使用现有 daemon client API。
- library surface 不公开 `daemon`、`cli`、`tui`、`overlay`、`platform`、`voice`、
  `hotkey`、`config`、`reload` 或 `ipc::server`。
- 仍无 Tauri workspace 或 GUI runtime 依赖，daemon/TUI 用户可见行为不变。

Phase 9g 验收：

- `client_api` 公开 daemon connection state、recoverable problem kind 和 retry delay helper。
- helper 是纯函数，不连接 IPC、不启动 daemon、不读取配置/history。
- `PROTO_VERSION` 仍为 2；不新增 IPC command/event，不新增 Tauri/WRY/WebView 依赖。

Phase 9h 验收：

- `client_api` 公开 GUI backend event 类型和纯 bridge helper。
- daemon event bridge 复用 `classify_first_screen_event()`；连接状态/problem bridge 不连接 IPC、
  不启动 daemon、不读配置/history。
- `PROTO_VERSION` 仍为 2；不新增 IPC command/event，不新增 Tauri/WRY/WebView 依赖。

Phase 9i 验收：

- `client_api` 公开首屏 timing/readiness 类型和纯 helper。
- helper 只接收调用方提供的毫秒时间戳和既有 `FirstScreenEvent`，不读系统时间、不启动
  runtime、不连接 IPC、不写 metrics sink。
- `PROTO_VERSION` 仍为 2；不新增 IPC command/event，不新增 Tauri/WRY/WebView 依赖。

Phase 9j 验收：

- `gui.md` 明确 Tauri capabilities/permissions 的 PoC 授权边界：主 window/webview、最小
  command 权限、scopes 评审和宽权限禁用默认策略。
- 仍无 `src-tauri/**` workspace 文件，`Cargo.toml` 不新增 Tauri/WRY/WebView 依赖。
- 不新增 IPC command/event，不改变 TUI/CLI 行为，不启动 daemon/GUI。

Phase 9k 验收：

- `gui.md` 明确最小 workspace 创建前的允许新增路径、禁止 scope creep、自动验收和 release
  指标清单。
- 仍无 `src-tauri/**` workspace 文件，`Cargo.toml` 不新增 Tauri/WRY/WebView 依赖。
- 不运行 `tauri build` / `tauri bundle`，不启动 daemon/GUI，不新增 IPC command/event。

Phase 9l 验收：

- `gui.md` 明确 reconnect supervisor ownership、取消 owner、session generation、recoverable
  problem 范围、metrics ownership 和 shared `client_api` 纯边界。
- 仍无 `src-tauri/**` workspace 文件，`Cargo.toml` 不新增 Tauri/WRY/WebView 依赖。
- 不实现 runtime loop，不启动 daemon/GUI，不新增 IPC command/event。

Phase 9m 验收：

- `src-tauri/**` 只包含最小 Tauri skeleton 文件，且 `src-tauri/Cargo.toml` 通过 path dependency
  依赖根 crate `shuohua`。
- root `Cargo.toml`、`src/daemon/**`、`src/tui/**` 和 `src/client_api.rs` 仍不含 Tauri/WRY/WebView
  runtime token。
- 不运行 `tauri dev` / `tauri build` / `tauri bundle`，不启动 daemon/GUI，不新增 IPC
  command/event。

Phase 9n 验收：

- `src-tauri/src/lib.rs` 注册最小 metadata command，并通过 `tauri::generate_handler!` 接入
  builder；command 返回静态 shell metadata，不连接 daemon IPC。
- `gui-dist/index.html` 存在，作为 `frontendDist` 的最小静态 placeholder；不新增 frontend
  package manager、dev server config 或 build output 目录。
- root `Cargo.toml`、`src/daemon/**`、`src/tui/**` 和 `src/client_api.rs` 仍不含 Tauri/WRY/WebView
  runtime token；`src-tauri/**` 不出现 `connect_default`、`tokio::spawn`、`Command::` 或
  `Event::`。
- 不运行 `tauri dev` / `tauri build` / `tauri bundle`，不启动 daemon/GUI，不新增 IPC
  command/event。

Phase 9o 验收：

- `src-tauri/src/lib.rs` 注册 `gui_first_screen_request_plan` command，并通过 shared
  `client_api::first_screen_commands()` 生成 summary。
- command 返回 summary，不创建 `DaemonClient`，不调用 `connect_default()`，不出现
  `send_command`、`subscribe_events`、`tokio::spawn` 或 reconnect timer。
- `gui-dist/index.html` 只展示 request plan summary，不实现真实 Status/History/Diagnostics
  view model。
- root `Cargo.toml`、daemon、TUI 和 shared `client_api` 仍不含 Tauri/WRY/WebView runtime token。

参考资料：

- [Tauri v2 develop guide](https://v2.tauri.app/develop/)
- [Tauri JavaScript Window API](https://v2.tauri.app/reference/javascript/api/namespacewindow/)
- [Tauri CLI build command](https://v2.tauri.app/reference/cli/#build)
- [Tauri CLI bundle command](https://v2.tauri.app/reference/cli/#bundle)
- [Tauri sidecar guide](https://v2.tauri.app/learn/sidecar-nodejs/)
- [Tauri v2 permissions system](https://v2.tauri.app/blog/tauri-20/#the-allowlist-is-dead-long-live-the-allowlist)
