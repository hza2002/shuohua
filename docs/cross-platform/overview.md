# Cross-Platform Overview

## 目标

shuohua 要从 macOS-first 工具演进成三端桌面应用：macOS、Windows、Linux
提供相同的核心语音输入体验、相同的数据模型和尽量一致的配置文件。平台差异主要体现在
overlay 材质、窗口系统能力、权限诊断和服务管理方式上。

用户视角的目标：

- 一份 `config.toml`、profile、ASR/post 配置和 theme 尽量三端可同步。
- daemon 常驻开销低；核心热路径不加载 WebView 或桌面 GUI runtime。
- 录音热路径不依赖按需 client。
- overlay 在每个平台尽量原生，效果可降级但文字必须清楚可读。

## 进程模型

长期形态仍然是 daemon + clients：

```text
shuo daemon              常驻核心：hotkey / recording / ASR / post / history / IPC
shuo CLI/TUI             按需 client：状态、历史、配置、服务管理
native overlay renderer  daemon 内或 renderer host：低延迟状态浮层
```

TUI 和外部工具通过统一 IPC protocol 与 daemon 通信；按需 client 不能改变 daemon 的低开销常驻原则。

## 分层边界

| 层 | 共享 | 平台化 |
|---|---|---|
| Core | config/profile/asr/post/history/state/schema | 无 |
| IPC protocol | JSON-line command/event model | transport：UDS / Named Pipe |
| Service | user-facing command contract | launchd / systemd user / Windows logon task |
| Desktop capability | capability/status model | permission、hotkey、injection、active app |
| Overlay | command/model/layout/theme tokens | AppKit / Win32-DWM / Wayland backend |

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
  的 capability summary 会用 renderer snapshot 覆盖 overlay 条目；TUI 未接入。
- Phase 7a Windows Overlay PoC Baseline：已完成文档调研，基于 Microsoft 文档记录 Win32
  overlay window、layered alpha、topmost/no-activate/tool window、hit-test 穿透和
  Mica/DWM backdrop 的验证顺序；尚未写 Windows backend。
- Phase 8a Linux Wayland Overlay PoC Baseline：已完成文档调研，基于 Wayland/wlr
  layer-shell/KDE/GNOME 资料记录 compositor 支持矩阵、screen-anchor fallback、
  input passthrough 和 material fallback 验证顺序；尚未写 Linux backend。
- Phase 9 GUI PoC Archive：GUI/Tauri PoC 已移出当前 runtime 分支，归档在 `feat/gui-poc-archive`。
  当前分支不携带 GUI workspace、静态 frontend、GUI client helper 或 library split 代码；后续 GUI 产品阶段
  需要基于最新 `main` 重新取舍。
- Phase 10c Linux Docker Cross Check Baseline：已完成自动验证，macOS 主机通过 Docker/cross 执行
  `x86_64-unknown-linux-gnu` compile check；该基线只证明 Linux 编译/cfg 边界，不代表 runtime
  可用。
- Phase 10d Linux Compile-Time Capability Sync：已完成自动验证，Linux 静态 capability snapshot
  如实标记已 compile-checked 的 UDS、lock file、process probe 和 ALSA audio capture；
  service manager 仍未实现。
- Phase 10e Linux Systemd User Dry-Run Status Skeleton：已完成自动验证，Linux service backend
  可以生成 systemd user unit path/body 并在 `service status` 输出 dry-run 信息；不写 unit 文件、
  不调用 `systemctl --user`，install/start/stop/restart 仍 unsupported。
- Phase 10f Linux Service Manager Capability Sync：已完成自动验证，Linux `service.manager`
  静态 capability 从 generic unsupported 同步为 `partial/systemd_user_dry_run/dry_run_status_only`；
  这只代表 dry-run/status skeleton，不代表 systemd runtime 已可用。
- Phase 10g Path Open/Reveal Facade：已完成自动验证，TUI config/audio open/reveal 改走
  `platform::path`；macOS 保持 `open` / `open -R`，Linux 使用 `xdg-open` 并把 reveal 降级为打开
  父目录，Windows 仍 unsupported。
- Phase 10h Windows Path Open/Reveal Compile Backend：已完成自动验证，Windows `platform::path`
  使用 `explorer.exe` / `/select,` 作为 compile backend，`path.open_reveal` 标记为
  `partial/explorer/runtime_not_verified`；真实 shell 行为仍需 Windows VM/实机验证。
- Phase 10i Audio Convert Facade：已完成自动验证，retained audio conversion 改走
  `platform::audio_convert`；macOS 保持 `/usr/bin/afconvert` 参数和 cleanup 语义，Linux/Windows
  暂时明确 unsupported，直到选定并实机验证转换 backend。
- Phase 10j Windows Lifecycle Primitive Compile Backend：已完成自动验证，Windows
  `platform::lifecycle` 使用 Win32 named mutex / `OpenProcess` 作为 compile backend，
  `daemon.single_instance` 和 `process.probe` 标记为 `partial/runtime_not_verified`；真实 daemon
  lifecycle 仍需 Windows VM/实机验证。
- Phase 10k Windows Service Manager Dry-Run Status Skeleton：已完成自动验证，Windows
  `platform::service` 可以输出 user-session service/logon-task dry-run status，`service.manager`
  标记为 `partial/windows_user_dry_run/dry_run_status_only`；install/start/stop/restart 仍 unsupported。
- Phase 10l Non-macOS Desktop Capability Truthfulness：已完成自动验证，Linux/Windows
  desktop hotkey/clipboard/text injection/active app/permissions 的静态 capability 与当前 facade
  行为同步；不实现新的桌面 runtime backend。
- Phase 10aj Windows Active App Identity Diagnostics：已完成 Windows 原生验证，Windows
  `platform::desktop::frontmost_app()` 现在通过 foreground window owner process 暴露
  `windows_exe_name`，`profile.routes.<profile>.windows.exe_name` 有真实 runtime 输入；AUMID 和
  Linux active app backend 仍未实现，Windows `desktop.active_app` 保持
  `partial/foreground_window_process_exe/exe_name_only`。

1. 定义 platform capability/status 模型，让 unsupported、partial、degraded 和 permission
   failure 都能被 CLI/TUI/doctor 和外部 client 解释。
2. 稳定配置和 theme 的跨平台规则，避免把平台视觉细节放进主配置。
3. 抽 IPC transport，保持 JSON-line protocol 不变。
4. 抽 daemon 单实例、process probing 和 service manager。
5. 抽 desktop capability：hotkey、clipboard、text injection、active app、permissions。
6. 抽 overlay renderer 边界，保留 macOS renderer，新增 Windows/Linux renderer 骨架。
7. 优先完成 Windows/Linux 原生 overlay 和非 macOS TUI/overlay 可用性。

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
- daemon 常驻路径不能引入 WebView 或桌面 GUI runtime。
- 日志、诊断和支持 bundle 不能记录敏感正文。
