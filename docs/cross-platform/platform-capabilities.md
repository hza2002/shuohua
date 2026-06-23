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

非 macOS 默认不实现 backend。快照应为上述 capability 返回 `unsupported`，
platform 按编译目标设置为 Linux/Windows/Unknown，reason 使用 `backend_not_implemented`。
后续阶段已落地的编译级 backend 可以覆盖默认项，但必须如实表达未实机验证的范围。

- Windows `ipc.transport`：`partial`，backend `named_pipe`，reason `runtime_not_verified`。
  这表示 Tokio Named Pipe transport 已通过 Windows target compile check，但 connect/bind/accept、
  ACL/security descriptor、multi-user 隔离和 pipe busy 行为仍需 Windows 实机/VM 验证。

## Phase 10d Linux Compile-Time Capability Sync

Phase 10d 只同步 Linux target 已经具备编译边界的静态 capability，不实现新的 runtime backend：

- `ipc.transport`：`available`，backend `unix_domain_socket`。Linux 复用 Unix domain socket
  transport；当前只通过 Docker/cross compile check，不代表真实 Linux daemon/client runtime
  已验证。
- `daemon.single_instance`：`available`，backend `lock_file`。Linux 复用 Unix lock file + `flock`
  lifecycle primitive；当前只代表编译边界已存在。
- `process.probe`：`available`，backend `unix_process_probe`。Linux 复用 `kill(pid, 0)` probe；
  当前未在 Linux 实机验证权限和 pid namespace 行为。
- `service.manager`：`unsupported`，backend `systemd_user_skeleton`，reason `backend_not_implemented`。
  Phase 10d 不实现 systemd user install/start/stop/status。
- `audio.capture`：`partial`，backend `cpal_alsa`，reason `compile_checked`。Docker/cross 已通过
  ALSA sysroot 编译检查；真实 input device enumeration、permission、default device selection
  仍需 Linux 实机验证。
- `audio.convert`：`unsupported`，backend `none`。当前 retained audio conversion 仍依赖 macOS
  `afconvert` 路径，Linux conversion backend 后续再设计。

其他 Linux desktop、overlay、path open/reveal capability 继续按对应 skeleton/unsupported 阶段推进。

## Phase 10f Linux Service Manager Capability Sync

Phase 10e 后，Linux service backend 已经有 dry-run/status skeleton，因此静态 capability 不应继续
把 `service.manager` 描述为完全未实现：

- `service.manager`：`partial`，backend `systemd_user_dry_run`，reason `dry_run_status_only`。
- summary 应明确只支持生成 systemd user unit path/body 和 dry-run status。
- next step 应指向真实 Linux 环境中的 systemd user install/start/stop/status 验证。

Phase 10f 仍不实现 service install/start/stop/restart，不写 unit 文件，不调用 `systemctl --user`，
不声明 Linux service runtime 可用。

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

## Phase 6 Overlay Renderer Capability

Phase 6b 将 overlay renderer 相关静态快照收敛到 `overlay::renderer`：

- `platform::capability` 继续拥有共享 status/id/platform 类型和全局静态快照。
- `overlay::renderer::renderer_capabilities()` 只返回 overlay renderer surface：
  `overlay.renderer`、`overlay.material`、`overlay.always_on_top`、
  `overlay.input_passthrough`、`overlay.window_anchor`。
- 该函数不创建窗口、不读取业务配置、不执行权限或 compositor probe；它只是给后续
  doctor/TUI/GUI 接入 renderer 降级信息预留稳定形状。
- macOS 值与 Phase 1 全局快照保持一致；非 macOS 返回 unsupported/backend_not_implemented。

Phase 6c 先只接入 doctor：

- `shuo doctor` 的 capability summary 先读全局静态快照，再用
  `overlay::renderer_capabilities()` 覆盖同 `CapabilityId` 的 overlay 条目。
- doctor 仍只打印非阻断 summary，不改变错误/警告计数或退出码。
- TUI/GUI 消费方式留到单独阶段设计，不让 daemon 热路径或业务层直接依赖 renderer snapshot。

## Phase 1 非目标

- 不抽 hotkey backend。
- 不改 IPC protocol 或 transport。
- 不改变 doctor/TUI 的控制流或错误语义。
- 不实现 Linux/Windows backend。
- 不把 capability 写入 history/trace。
