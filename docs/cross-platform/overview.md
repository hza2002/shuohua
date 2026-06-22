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
