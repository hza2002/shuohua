# Cross-Platform Overview

## 目标

shuohua 要从 macOS-first 工具演进成三端桌面应用：macOS、Windows、Linux
提供相同的核心语音输入体验、相同的数据模型和尽量一致的配置文件。平台差异主要体现在
overlay 材质、窗口系统能力、权限诊断和服务管理方式上。

用户视角的目标：

- 一份 `config.toml`、profile、ASR/post 配置和 theme 尽量三端可同步。
- daemon 常驻开销低；未打开 GUI 时不加载 WebView。
- 录音热路径不依赖 GUI。
- overlay 在每个平台尽量原生，效果可降级但文字必须清楚可读。
- GUI 用一套 Tauri UI，先覆盖 TUI，后续承载复杂配置、历史浏览、诊断、登录和 onboarding。

## 进程模型

长期形态仍然是 daemon + clients：

```text
shuo daemon              常驻核心：hotkey / recording / ASR / post / history / IPC
shuo CLI/TUI             按需 client：状态、历史、配置、服务管理
Shuo GUI (Tauri)         按需 client：完整桌面 GUI，不常驻 daemon 热路径
native overlay renderer  daemon 内或 renderer host：低延迟状态浮层
```

GUI 和 TUI 都通过统一 client API 与 daemon 通信。GUI 可以打包成完整 App，
但不改变 daemon 的低开销常驻原则。

## 分层边界

| 层 | 共享 | 平台化 |
|---|---|---|
| Core | config/profile/asr/post/history/state/schema | 无 |
| IPC protocol | JSON-line command/event model | transport：UDS / Named Pipe |
| Service | user-facing command contract | launchd / systemd user / Windows logon task |
| Desktop capability | capability/status model | permission、hotkey、injection、active app |
| Overlay | command/model/layout/theme tokens | AppKit / Win32-DWM / Wayland backend |
| GUI | Tauri frontend + client API | packaging/signing/platform plugins |

## 开发顺序

当前状态：

- Phase 0 Baseline Audit：已提交自动基线和 macOS 手动 checklist；真实 macOS 手动体验仍需用户验证。
- Phase 1 Platform Capability Model：已提交共享 capability/status 静态模型和 doctor 非阻断摘要。
- Phase 2 Config And Theme Cross-Platform Rules：已完成自动验证，稳定了 config/theme schema 和 starter template。
- Phase 3 IPC Transport Boundary：已完成自动验证，JSON-line protocol 保持不变，IPC transport
  集中到 `ipc::transport`。
- Phase 4 Single Instance, Process Probe And Service Manager：已完成自动验证，daemon lock
  和 process probe 集中到 `platform::lifecycle`，用户会话 service manager 集中到
  `platform::service`；macOS launchd 行为保持不变。
- Phase 5 Desktop Capability Boundary：已完成自动验证，active app、clipboard、
  text injection 和 permission primitives 的业务入口收敛到 `platform::desktop`；
  hotkey provider 启动边界收敛到 `platform::hotkey`。macOS CGEventTap callback、wire
  format、tracker 和 suppress 行为保持不变。
- Phase 6a Overlay Renderer Facade：已完成自动验证，renderer backend 选择集中到
  `overlay::renderer`；macOS AppKit renderer 行为保持不变。
- Phase 6b Overlay Renderer Capability Skeleton：已完成自动验证，overlay renderer
  capability/status 静态快照集中到 `overlay::renderer`；Windows/Linux renderer 未实现。
- Phase 6c Overlay Renderer Capability Consumption：已完成自动验证，`shuo doctor`
  的 capability summary 会用 renderer snapshot 覆盖 overlay 条目；TUI/GUI 未接入。
- Phase 7a Windows Overlay PoC Baseline：已完成文档调研，基于 Microsoft 文档记录 Win32
  overlay window、layered alpha、topmost/no-activate/tool window、hit-test 穿透和
  Mica/DWM backdrop 的验证顺序；尚未写 Windows backend。
- Phase 8a Linux Wayland Overlay PoC Baseline：已完成文档调研，基于 Wayland/wlr
  layer-shell/KDE/GNOME 资料记录 compositor 支持矩阵、screen-anchor fallback、
  input passthrough 和 material fallback 验证顺序；尚未写 Linux backend。
- Phase 9a Tauri GUI PoC Baseline：已完成文档调研，基于 Tauri v2 文档记录 GUI 独立
  client、command/event 桥接、permissions/capabilities、sidecar 非默认路线和
  build/bundle 指标采集 checklist；尚未写 GUI app。
- Phase 9b GUI Client API Boundary：已完成自动验证，新增 GUI/TUI 复用的 daemon client
  入口，TUI 通过该边界引用 daemon client；继续复用现有 JSON-line IPC，未引入
  Tauri/WebView runtime。
- Phase 9c GUI First Screen Client Helpers：已完成自动验证，在 shared client API 内增加首屏
  request helper 和 response classifier，仍未创建 Tauri workspace、未改 IPC protocol。
- Phase 9d GUI Library Boundary Preconditions：已完成自动验证，记录当前 binary crate 限制和
  后续 library split 前置条件，避免在没有 library surface 前创建 Tauri workspace。
- Phase 9e GUI Library Split Audit Baseline：已完成自动验证，记录最小 library surface、
  `history`/`state`/Unix-only transport 阻塞点，以及创建 Tauri workspace 前的依赖方向。
- Phase 9f GUI Minimal Library Split：已完成自动验证，新增只暴露 daemon client/protocol
  surface 和必要 DTO 的 library target；仍未创建 Tauri workspace，daemon/CLI/TUI 行为不变。
- Phase 9g GUI Client Reconnect State Skeleton：已完成自动验证，在 shared client API 里新增
  GUI 可复用的 daemon connection/reconnect 状态、recoverable problem kind 和 bounded retry
  helper；未新增 IPC protocol 或 GUI runtime。
- Phase 9h GUI Backend Event Bridge Skeleton：已完成自动验证，在 shared client API 里新增
  GUI backend 可转发事件形状，复用既有 daemon event 和 connection state；未新增 IPC protocol
  或 GUI runtime。
- Phase 9i GUI First Screen Metrics Timing Model：已完成自动验证，在 shared client API 里新增
  首屏 timing/readiness 纯模型；不创建 Tauri workspace、不新增 IPC protocol 或 GUI runtime。
- Phase 9j Tauri Permissions Capabilities Preflight：已完成自动验证，在创建 Tauri workspace 前
  记录主 window/webview 最小 capability、command permissions 和宽权限禁用边界。
- Phase 9k Tauri Workspace Pre-Creation Acceptance Checklist：已完成自动验证，在创建 Tauri
  workspace 前收口允许新增路径、禁止 scope creep、自动验收和 release 指标清单。
- Phase 9l GUI Reconnect Supervisor Ownership：已完成自动验证，在创建 GUI runtime 前记录
  reconnect supervisor 的所有权、取消语义、session generation 和 metrics ownership。
- Phase 9m Minimal Tauri Workspace Skeleton：已完成自动验证，新增只包含最小 Tauri app
  skeleton 的 `src-tauri/**`，同时保持 daemon/TUI/root crate 不依赖 GUI runtime。
- Phase 9n Minimal GUI Backend Shell：已完成自动验证，新增本地 metadata command 和静态
  `gui-dist/index.html` placeholder，不连接 daemon、不实现页面 view model。
- Phase 9o GUI First-Screen Request Plan：已完成自动验证，在 GUI backend 暴露首屏请求计划
  command，复用 shared client API 但不连接 daemon、不发送 IPC。
- Phase 9p GUI Daemon Status Snapshot Shape：已完成自动验证，在 GUI backend 暴露静态 daemon
  status snapshot response shape，不连接 daemon、不发送 IPC、不启动 reconnect loop。
- Phase 9q GUI Daemon Status Event Mapper：已完成自动验证，在 GUI backend 增加纯
  `Event::DaemonStatus` 到 status snapshot shape 的 mapper，不连接 daemon、不发送 IPC。
- Phase 9r GUI Daemon Status One-Shot Request：已完成自动验证，在 GUI backend 增加显式调用的
  one-shot daemon status request command，不订阅、不重连、不自动启动 daemon。
- Phase 9s GUI History Summary One-Shot Request：已完成自动验证，在 GUI backend 增加显式调用的
  one-shot history summary request command，复用既有 `GetHistory`/`GetHistoryStats` IPC，不订阅、
  不重连、不自动启动 daemon、不实现完整 History view model。
- Phase 9t GUI First-Screen Summary One-Shot Request：已完成自动验证，在 GUI backend 增加显式调用的
  one-shot first-screen summary request command，复用既有 status/history IPC，一次连接返回组合
  summary，不订阅、不重连、不自动启动 daemon。
- Phase 9u GUI First-Screen Summary Request Timing：已完成自动验证，在 9t 的显式 one-shot
  first-screen summary response 上附带 GUI backend 本地 timing，不改 IPC、不订阅、不启动 timer task。
- Phase 9v GUI First-Screen Explicit Refresh Shape：已完成自动验证，在 GUI backend 暴露手动刷新入口的
  静态 shape，不自动调用真实 one-shot request、不订阅、不重连、不改 IPC。
- Phase 9w GUI First-Screen Readiness Timing Display Shape：已完成自动验证，在 GUI backend 暴露首屏
  readiness/timing 空态展示 shape，不自动请求、不启动 timer、不改 IPC。
- Phase 9x GUI First-Screen Offline Error Display Shape：已完成自动验证，在 GUI backend 暴露首屏
  daemon offline / recoverable error 静态展示 shape，不自动恢复、不启动 service、不改 IPC。
- Phase 9y GUI First-Screen Command Invocation Policy Shape：已完成自动验证，在 GUI backend 暴露首屏
  command 自动/显式调用策略 shape，不自动请求、不改 IPC。
- Phase 9z GUI First-Screen Explicit Refresh Affordance Shape：已完成自动验证，在 GUI backend 暴露首屏
  手动刷新控件静态展示 shape，不接真实点击、不自动请求、不改 IPC。
- Phase 9aa GUI First-Screen Explicit Refresh Click Wiring：已完成自动验证，placeholder 只在用户点击后调用
  既有 one-shot summary command，不自动请求、不订阅、不改 IPC。
- Phase 9ab GUI First-Screen Explicit Refresh Result Projection：已完成自动验证，placeholder 只在用户点击成功后
  投影 summary 到现有文本字段，不新增请求、不订阅、不改 IPC。
- Phase 9ac GUI First-Screen Explicit Refresh Error Projection：已完成自动验证，placeholder 只在用户点击失败后
  投影 request error 到现有文本字段，不新增请求、不订阅、不改 IPC。
- Phase 9ad GUI First-Screen Explicit Refresh Success Clears Offline Display：已完成自动验证，placeholder 只在用户点击成功后
  清理 stale offline/error 文本，不新增请求、不订阅、不改 IPC。
- Phase 9ae GUI Command Permission And Init Error Visibility：已完成自动验证，Tauri capability 显式授权
  当前 placeholder frontend invoke 的 application command，初始化失败会显示到现有 action 字段；
  仍不订阅 daemon event、不流式显示 recording state、不自动请求。
- Phase 9af GUI Static Frontend Global Tauri API：已完成自动验证，为无 bundler 静态 HTML 启用
  `withGlobalTauri`，并在 Tauri invoke API 缺失时显示 `tauri-api-missing`；仍不订阅 daemon event。
- Phase 9ag GUI Manual Refresh Readable Summary：已完成自动验证，placeholder 在显式 Refresh 后显示
  connected/state/history/latest/timing/error 摘要；仍不订阅 daemon event、不自动请求。
- Phase 9ah GUI Frontend First-Screen View Model Preflight：已完成自动验证，静态 HTML 内维护本地
  `firstScreenViewModel` 作为后续 Tauri event subscription 的前端落点；仍不订阅 daemon event。
- Phase 9ai GUI Backend Daemon Event Stream Bridge：已完成自动验证，Tauri backend 暴露显式
  `gui_start_daemon_event_stream` command，启动 GUI-owned background task 连接既有 daemon IPC、
  发送 `Command::Subscribe` 并把 first-screen daemon event 转成 Tauri event；仍不实现 reconnect
  supervisor、service management、daemon auto-start 或 frontend 自动订阅。
- Phase 9aj GUI Frontend Daemon Event Listener Wiring：已完成自动验证，静态 HTML 初始化时注册
  `shuohua://daemon-event` listener 并显式启动 backend bridge，把 incoming event payload 投影到
  `firstScreenViewModel` / DOM；仍不实现 reconnect supervisor、service management、daemon auto-start
  或 recording controls。下一步需要真实 macOS 手动验证录音期间 GUI 是否自动变化。

1. 定义 platform capability/status 模型，让 unsupported、partial、degraded 和 permission
   failure 都能被 CLI/TUI/GUI/doctor 解释。
2. 稳定配置和 theme 的跨平台规则，避免把平台视觉细节放进主配置。
3. 抽 IPC transport，保持 JSON-line protocol 不变。
4. 抽 daemon 单实例、process probing 和 service manager。
5. 抽 desktop capability：hotkey、clipboard、text injection、active app、permissions。
6. 抽 overlay renderer 边界，保留 macOS renderer，新增 Windows/Linux renderer 骨架。
7. 做 Tauri GUI client，先覆盖 TUI 能力，再扩展复杂 GUI 功能。

每一步都要保持 macOS 当前行为不回退。新增平台 backend 前，先把共享接口和 capability
诊断跑通。

## 平台支持基线

- macOS：核心 macOS 15+；Liquid Glass 优先 macOS 26+；旧系统 fallback 到普通 glass/tint。
- Windows：优先 Windows 11；Windows 10 可运行但高级材质可降级。
- Linux：Wayland-first；X11 预留 backend 位置，成本过高时允许不支持。

Linux 不承诺所有 compositor 行为一致。核心录音、ASR、post、history 必须可用；overlay
高级窗口能力通过 capability probe 决定。

## 修订原则

这些文档描述当前最佳判断，不要求实现按早期猜测硬走。开发中如果发现平台 API、
依赖成本或用户体验与文档不一致，应先修订相关文档，再继续实现。

硬性保护项只有：

- macOS 当前可用功能不能回退。
- JSON-line 协议、history schema、配置 schema 的对外变化必须有迁移说明。
- daemon 常驻路径不能引入 GUI/WebView 运行时。
- 日志、诊断和支持 bundle 不能记录敏感正文。
