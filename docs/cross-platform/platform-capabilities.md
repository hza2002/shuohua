# Platform Capabilities

## 目标

平台差异需要显式建模。调用方不应通过字符串错误猜测某能力是否支持，也不应把
macOS 的实现细节投射到 Windows/Linux。

## 状态模型

Phase 1 先落地共享只读模型，不改变 daemon/TUI 行为，也不改变 doctor 的错误/警告语义。
doctor 可以打印非阻断 summary，状态类型应位于 `src/platform/capability.rs`，由
`src/platform/mod.rs` 暴露给 crate 内部调用方。

共享状态：

| 状态 | 含义 |
|---|---|
| `available` | 当前环境可用 |
| `unsupported` | 该平台/backend 不支持 |
| `unavailable` | 理论支持，但当前缺权限、依赖或运行时条件 |
| `partial` | 可用但有明确限制 |
| `degraded` | 已启用 fallback |
| `unknown` | probe 失败，无法确定 |

每个状态应携带：

- capability id。
- platform/backend。
- human-readable summary。
- machine-readable reason code。
- optional next step。

Phase 1 的字段约定：

- `CapabilityId` 使用静态枚举，不用自由字符串，避免调用方拼错。
- `CapabilityStatusKind` 使用上述六种状态，并提供稳定的 snake_case 字符串。
- `PlatformKind` 至少包含 `Macos`、`Linux`、`Windows`、`Unknown`。
- `CapabilityStatus` 包含 `id`、`platform`、`backend`、`status`、`summary`、
  `reason`、`next_step`。
- `backend`、`summary`、`reason`、`next_step` 使用 `&'static str` / `Option<&'static str>`；
  Phase 1 不做本地化，doctor/TUI 后续接入时再决定展示文案来源。
- `current_platform_capabilities()` 返回当前平台的低风险静态快照；不做权限弹窗、不启动
  daemon、不打开 GUI、不 probe 高风险系统 API。

## 能力列表

第一批候选能力：

- `ipc.transport`
- `daemon.single_instance`
- `service.manager`
- `process.probe`
- `desktop.hotkey`
- `desktop.hotkey_suppression`
- `desktop.clipboard`
- `desktop.text_injection`
- `desktop.active_app`
- `desktop.permissions`
- `overlay.renderer`
- `overlay.material`
- `overlay.always_on_top`
- `overlay.input_passthrough`
- `overlay.window_anchor`
- `audio.capture`
- `audio.convert`
- `path.open_reveal`

## 消费方

- daemon startup：决定启用哪些 backend。
- doctor：展示问题和 next step。
- TUI/GUI：展示当前环境能力和降级。
- history/trace：记录低频诊断，不记录敏感正文。

Phase 1 只提供模型和快照函数；doctor/TUI 可以读取但不强依赖。后续阶段接入消费方时，
不得把 macOS 字符串错误当作 capability 判断来源。

## Phase 1 初始映射

macOS 当前快照先把现有能力映射为 current status：

- `ipc.transport`：`available`，backend `unix_domain_socket`。
- `daemon.single_instance`：`available`，backend `lock_file`。
- `service.manager`：`available`，backend `launchd_user_agent`。
- `process.probe`：`available`，backend `unix_process_probe`。
- `desktop.hotkey`：`available`，backend `cgeventtap`。
- `desktop.hotkey_suppression`：`available`，backend `cgeventtap_drop`。
- `desktop.clipboard`：`available`，backend `nspasteboard`.
- `desktop.text_injection`：`available`，backend `cgevent_paste`.
- `desktop.active_app`：`available`，backend `nsworkspace`.
- `desktop.permissions`：`available`，backend `accessibility_microphone`.
- `overlay.renderer`：`available`，backend `appkit_panel`.
- `overlay.material`：`degraded`，backend `appkit_glass`，reason 表达 Liquid Glass 可能
  fallback 到 HUD/tint。
- `overlay.always_on_top`：`available`，backend `nsstatuswindowlevel`.
- `overlay.input_passthrough`：`partial`，backend `nonactivating_panel`，reason 表达不抢焦点但
  仍由 renderer 负责具体鼠标策略。
- `overlay.window_anchor`：`degraded`，backend `accessibility_focused_window`，reason 表达取不到
  focused window 时退屏幕锚定。
- `audio.capture`：`available`，backend `cpal`.
- `audio.convert`：`available`，backend `afconvert`.
- `path.open_reveal`：`available`，backend `open_command`.

非 macOS Phase 1 暂不实现 backend。快照应为上述 capability 返回 `unsupported`，
platform 按编译目标设置为 Linux/Windows/Unknown，reason 使用 `backend_not_implemented`。

## 设计约束

- capability probe 不执行高风险动作。
- probe 不应阻塞 AppKit/Tauri/window callback。
- permission 诊断应平台化。
- unsupported 是正常状态，不是 panic。

## Phase 5 Desktop Facade

Phase 5 拆成两个小步，避免一次性改动 hotkey 热路径：

- Phase 5a：新增 `platform::desktop` 作为业务层统一入口，聚合 active app、clipboard、
  text injection 和 permission primitives。macOS 继续转发到现有 AppKit/CoreGraphics
  backend；非 macOS 返回 capability-aware unsupported 或 conservative default。
- Phase 5b：再评审 hotkey provider facade，保留 macOS CGEventTap 线程模型和 suppress
  down/up 配对语义，不在 5a 改 CGEventTap callback 或 `hotkey` 状态机。

Phase 5a 之后，voice、daemon、TUI 和 doctor 不应直接依赖 `platform::macos`、
`platform::{clipboard,autotype,permissions}` 或 `post::app_context::frontmost_app()`；
这些调用应通过 `platform::desktop`。`post::AppContext` 仍是 post pipeline 的数据模型，
但前台 App 查询属于 desktop capability，不属于 post processor 实现细节。

Phase 5b 只移动 hotkey provider 启动边界：

- `platform::hotkey` 拥有 hotkey provider backend 选择、OS thread spawn 和 unsupported
  fallback。
- macOS backend 继续调用 `hotkey::provider_darwin::run()`；CGEventTap callback、
  pipe wire format、`Suppressor` 和 `TrackerSet` 行为不变。
- `platform::daemon` 只保留 daemon runtime 需要的抽象 trait，不直接知道
  `provider_darwin`、thread 名称或非 macOS unsupported 文案。
- 不在 Phase 5b 实现 Linux/Windows global hotkey backend，也不引入跨平台 hotkey crate。

## Phase 1 非目标

- 不抽 hotkey backend。
- 不改 IPC protocol 或 transport。
- 不改变 doctor/TUI 的控制流或错误语义。
- 不实现 Linux/Windows backend。
- 不把 capability 写入 history/trace。
