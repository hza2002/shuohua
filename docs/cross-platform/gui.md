# GUI App

## 当前设计基线

GUI App 使用 Tauri 最新稳定版。Tauri GUI 是按需 client，不嵌入 daemon，不参与录音热路径。
daemon 未打开 GUI 时不加载 WebView。

Tauri 当前文档定位是 Rust backend + Web frontend，底层通过 WRY 使用各平台 WebView，
并提供 commands/events 与前端通信；bundler 覆盖 macOS、Windows、Linux。这个能力匹配
配置、历史、诊断、onboarding 等复杂 GUI。

如果后续 PoC 证明 Tauri 在目标平台的启动、内存、打包或系统集成成本不可接受，可以重新评估。
在有反证前，Tauri 是默认路线。

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

参考资料：

- [Tauri v2 develop guide](https://v2.tauri.app/develop/)
- [Tauri JavaScript Window API](https://v2.tauri.app/reference/javascript/api/namespacewindow/)
- [Tauri CLI build command](https://v2.tauri.app/reference/cli/#build)
- [Tauri CLI bundle command](https://v2.tauri.app/reference/cli/#bundle)
- [Tauri sidecar guide](https://v2.tauri.app/learn/sidecar-nodejs/)
- [Tauri v2 permissions system](https://v2.tauri.app/blog/tauri-20/#the-allowlist-is-dead-long-live-the-allowlist)
