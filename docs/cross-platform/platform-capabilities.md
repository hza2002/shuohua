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
  这表示 Tokio Named Pipe transport 已通过 Windows target compile check 和 same-user/elevation
  smoke，且 client connect 已收窄到 raw `CreateFileW` explicit access mask；cross-user 隔离和
  longer runtime soak 仍未完成。

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

## Phase 10g Path Open/Reveal Facade

TUI 中的 config/audio open/reveal 是用户会话能力，应走 platform facade，而不是业务层直接写
macOS `open` 命令：

- macOS：继续使用 `open` 和 `open -R`，行为不变。
- Linux：使用 `xdg-open` 打开文件或目录；reveal file fallback 到打开父目录，因此
  `path.open_reveal` 应标记为 `partial`，backend `xdg_open`，reason `reveal_opens_parent_dir`。
- Windows：本阶段不实现 shell open/reveal。

该阶段不改变 TUI 的路径安全检查，不改变 `$VISUAL` / `$EDITOR` 优先级，不把 path open/reveal
放进 daemon 常驻路径。

## Phase 10h Windows Path Open/Reveal Compile Backend

Windows `platform::path` backend 可先使用 `explorer.exe` 的命令行能力，作为 TUI open/reveal 的
compile backend：

- `open_path(path)`：`explorer.exe <path>`。
- `reveal_path(path)`：`explorer.exe /select,<path>`。
- `path.open_reveal`：`partial`，backend `explorer`，reason `runtime_not_verified`。

真实 Windows shell 行为、路径 quoting、UNC 路径、焦点和多用户会话仍需 Windows VM/实机验证。
Phase 10h 不引入 COM Shell API，也不实现 Windows daemon lifecycle、desktop injection 或 overlay。

## Phase 10i Audio Convert Facade

Retained audio conversion currently uses macOS `afconvert` directly from voice code. Phase 10i moves that
command behind `platform::audio_convert`:

- macOS：继续使用 `/usr/bin/afconvert`，参数和 cleanup 语义不变。
- Linux/Windows：暂时返回 unsupported；`audio.convert` capability 继续保持 unsupported，直到选定
  `ffmpeg`、`flac`/`lame`、纯 Rust encoder 或其他 backend，并在目标系统验证。

该阶段不改变 retained audio 文件命名、history schema、recorder WAV 写入、`record_audio = "off"`，
也不让 daemon 热路径引入外部转码依赖。

## Phase 10j Windows Lifecycle Primitive Compile Backend

Windows lifecycle no longer needs to be a pure unsupported placeholder for compile checks:

- `daemon.single_instance`：`partial`，backend `named_mutex`。`platform::lifecycle` uses a
  named Win32 mutex to model the daemon single-instance guard. Same-user and elevated/non-elevated smoke has
  passed. The backend maps `WAIT_ABANDONED` to an explicit warning/recovery path, but cross-user isolation and
  real crash/abandon smoke still need Windows validation.
- `process.probe`：`partial`，backend `open_process_probe`。`OpenProcess` is used as a compile backend
  for process existence probing, but PID reuse and permission behavior still need Windows validation.

Phase 10j 不实现 Windows service install/start/stop、smart fallback、daemon auto-start、Named Pipe ACL，
也不声明 Windows daemon lifecycle runtime-ready。

## Phase 10k Windows Service Manager Dry-Run Status Skeleton

Windows service manager support remains a design/runtime validation item, but the CLI can report a
structured dry-run status instead of falling through the generic unsupported backend:

- `service.manager`：`partial`，backend `windows_user_dry_run`，reason `dry_run_status_only`。
- `shuo service status` on Windows may print daemon-not-running plus a dry-run line that names the future
  user-session strategy and daemon command.
- `install` / `uninstall` / `start` / `stop` / `restart` still return unsupported.

Phase 10k 不调用 Task Scheduler、SCM、PowerShell 或 registry APIs，不写文件，不实现 smart fallback，
也不声明 Windows service lifecycle runtime-ready。

Phase 10ac adds Windows `service stop` as IPC shutdown only:

- `service.manager` remains `partial`; backend stays `windows_user_dry_run` because install/start/restart are
  still unsupported.
- reason becomes `ipc_stop_only` once `service stop` can send `Command::Shutdown` and wait for PID exit.
- This does not install, register, start, or manage a Task Scheduler/SCM service.

Phase 10ae adds explicit user-session `service start` / `restart`:

- `service.manager` remains `partial`; backend becomes `windows_user_session` because lifecycle control can
  start/stop/restart the current executable but still does not install or register startup integration.
- reason becomes `user_session_start_stop_only`.
- This does not call Task Scheduler, SCM, PowerShell, or registry APIs, and does not make Windows service
  lifecycle runtime-ready beyond the current user session.

## Phase 10l Non-macOS Desktop Capability Truthfulness

Linux/Windows desktop facade behavior is currently conservative and should be reflected explicitly in the
static capability snapshot:

- `desktop.hotkey` / `desktop.hotkey_suppression`：`unsupported`，backend `none`，reason
  `backend_not_implemented`。The non-macOS hotkey provider returns an explicit error.
- `desktop.clipboard` / `desktop.text_injection`：`unsupported`，backend `none`，reason
  `backend_not_implemented`。Clipboard write and paste injection remain macOS-only.
- `desktop.active_app`：`degraded`，backend `default_context`，reason `default_context_only`。The facade
  returns an empty/default `AppContext` instead of probing the foreground app.
- `desktop.permissions`：`unavailable`，backend `none`，reason `permission_probe_missing`。The facade has
  no Linux/Windows permission probe yet.

Phase 10l 只修正诊断 truthfulness，不实现 Linux/Windows hotkey、clipboard、text injection、
active app 或 permission runtime。

## Phase 10aj Windows Active App Identity Diagnostics

Windows `desktop.active_app` 第一版只实现 foreground window owner process lookup，用于给
`profile.routes.<profile>.windows.exe_name` 提供真实输入：

- backend：`foreground_window_process_exe`。
- status：`partial`，reason `exe_name_only`。
- 实现路径：`GetForegroundWindow` → `GetWindowThreadProcessId` →
  `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `QueryFullProcessImageNameW`，只暴露
  executable file name，不把完整进程路径写入 doctor、state、history 或 IPC。
- `app_user_model_id` 字段保留在 schema/model 中，但本阶段不实现 AUMID 查询，也不声明 packaged app
  identity route 已可用。
- `shuo doctor` 可以打印 `desktop.active_app.current`，用于 Windows runtime smoke；该诊断不启动录音、
  hotkey、overlay、clipboard 或 paste。

该阶段允许 Windows `exe_name` route 在后续 session start 时命中；如果 lookup 失败，仍应落回
`profile.default`。在真实录音 session 中验证 profile 命中、AUMID 查询和更多前台窗口类型之前，
不能把 `desktop.active_app` 升级为 `available`。

Phase 10ak extends the same diagnostic surface with `profile.route.current` in `shuo doctor`:

- It uses the current `AppContext` and the same platform `AppIdentity` conversion used by daemon session start.
- It reports whether route selection falls back to `profile.default`, matches exactly one route, or would fail
  because the app identity matches multiple profiles.
- It is a read-only config/identity diagnostic. It does not open profile provider runtime, start recording, or
  exercise hotkey/overlay/clipboard/paste.

## Phase 10al Windows Clipboard Write Backend

Windows `desktop.clipboard` 在 Phase 10al 只表达写剪贴板 backend 存在并做过同会话 smoke：

- `desktop.clipboard`：`partial`，backend `win32_clipboard_unicode`，reason `write_only_runtime_smoke`。
- backend 使用 Win32 `CF_UNICODETEXT`，不实现 clipboard restore，不读取旧剪贴板内容。
- `desktop.text_injection` 仍为 unsupported；本阶段不实现 `SendInput` paste，也不验证 record -> paste。
- 在 Notepad/browser/editor/terminal、UAC/elevation 边界和失败恢复都验证前，不能把 clipboard capability
  升级为 `available`。

## Phase 10am Windows Paste Injection Backend

Windows `desktop.text_injection` 在 Phase 10am 只表达 `SendInput` Ctrl+V backend 存在：

- `desktop.text_injection`：`partial`，backend `sendinput_ctrl_v`，reason `runtime_smoke_only`。
- backend 只发送 Control down、V down、V up、Control up，不负责选择目标窗口、不恢复 clipboard、
  不处理高完整性窗口/UAC 边界。
- `desktop.clipboard` 和 `desktop.text_injection` 是分开的能力；前者可成功但后者被目标 App 拒收。
- 在真实目标 App、elevation 边界和 full record -> paste session 验证前，不能升级为 `available`。

## Phase 10an Windows Low-Level Hotkey Backend

Windows `desktop.hotkey` / `desktop.hotkey_suppression` 在 Phase 10an 只表达 `WH_KEYBOARD_LL`
backend 存在并做过同会话 smoke：

- `desktop.hotkey`：`partial`，backend `wh_keyboard_ll`，reason `runtime_smoke_only`。
- `desktop.hotkey_suppression`：`partial`，backend `wh_keyboard_ll`，reason `runtime_smoke_only`。
- backend 运行在专用 OS 线程，callback 只写现有 `RawEvent` pipe wire format 并复用共享
  `Suppressor` 判断是否 drop foreground event。
- ignored runtime smoke 只验证 hook 可收到合成 F16 down/up；它不代表 hold-to-record、IME、真实
  foreground App、remote desktop 或 UAC/elevation 边界可用。
- 在真实目标 App 和完整 record -> paste session 验证前，不能升级为 `available`。

## Phase 10ao Windows Minimal Overlay Backend

Windows overlay 在 Phase 10ao 只表达最小 Win32 renderer 可以创建窗口并消费 `OverlayCmd`：

- `overlay.renderer`：`partial`，backend `win32_overlay_minimal`，reason `runtime_smoke_only`。
- `overlay.material`：`degraded`，backend `win32_overlay_minimal`，reason `translucent_fallback_only`。
- `overlay.always_on_top`：`partial`，backend `win32_overlay_minimal`，reason `runtime_smoke_only`。
- `overlay.input_passthrough`：`partial`，backend `win32_overlay_minimal`，reason `runtime_smoke_only`。
- `overlay.window_anchor`：`degraded`，backend `win32_overlay_minimal`，reason `screen_anchor_only`。
- 该 backend 不使用 Tauri/WebView，不引入 Direct2D/Skia/wgpu；先用 Win32/GDI 做 translucent
  layered window、basic text、show/hide/quit。
- 在真实 foreground App、UAC/fullscreen/multi-monitor、mouse/touch/pen passthrough 和最终视觉质量验证前，
  不能升级为 `available`。

## Phase 10ap Windows Overlay DPI And Font Baseline

Phase 10ap does not change Windows overlay capability levels, but narrows the reason behind visual mismatch:

- `win32_overlay_minimal` now scales window size, placement, text rectangles, and GDI font sizes by the current
  window DPI.
- Placement uses the Windows work area instead of raw primary-screen bounds.
- Text uses the platform UI font path (`Segoe UI`) as a DPI-scaled GDI baseline.
- macOS currently uses AppKit system fonts, not a hard JetBrains Mono or bundled SF Pro dependency.
- Capability remains partial/degraded until per-monitor secondary-display behavior, DirectWrite/Direct2D text,
  material/shadow/rounding, fullscreen/UAC behavior, and final visual QA are complete.

## Phase 10aq Windows Overlay Rounded GDI Baseline

Phase 10aq narrows the visual gap but does not change Windows overlay capability levels:

- `win32_overlay_minimal` now applies `overlay.surface.corner_radius` with `CreateRoundRectRgn` /
  `SetWindowRgn`.
- Layered-window opacity uses shared `overlay.surface.background_alpha`.
- Text creation requests `CLEARTYPE_QUALITY`, but the backend still uses GDI `DrawTextW`; this is not
  DirectWrite/Direct2D parity.
- `overlay.renderer` remains `partial`; `overlay.material` and `overlay.window_anchor` remain `degraded` until
  DirectWrite/Direct2D or equivalent text quality, material/shadow, focused anchoring, fullscreen/UAC, and
  multi-monitor visual QA are complete.

## Phase 10ar Windows Direct2D/DirectWrite Renderer Foundation

Phase 10ar changes the renderer implementation but does not upgrade capability levels yet:

- Windows overlay text and rounded-surface drawing now have a Direct2D/DirectWrite path isolated under the Windows
  overlay backend.
- The existing Win32 window shell still owns topmost/no-activate/tool-window/layered/click-through behavior.
- GDI remains a fallback if Direct2D/DirectWrite initialization or painting fails.
- This phase deliberately avoids DirectComposition, `UpdateLayeredWindow` per-pixel surfaces, Acrylic/Mica,
  shadows, and animation until the text renderer foundation is stable.
- `overlay.renderer` remains `partial`; `overlay.material` and `overlay.window_anchor` remain `degraded` until
  user-visible text QA, real foreground apps, fullscreen/UAC, multi-monitor behavior, and material/shadow decisions
  are complete.

## Phase 10as Windows Per-Pixel Layered Surface

Phase 10as changes Windows overlay compositing but does not upgrade capability levels yet:

- The Direct2D renderer now renders into a 32bpp premultiplied-alpha DIB and publishes it with
  `UpdateLayeredWindow` / `AC_SRC_ALPHA`.
- This removes Direct2D from the previous global `SetLayeredWindowAttributes` alpha path. Background pixels stay
  translucent while text keeps solid text alpha.
- GDI remains a fallback when Direct2D/per-pixel setup fails, so the fallback path may still use global layered alpha.
- `overlay.renderer` remains `partial`; `overlay.material` and `overlay.window_anchor` remain `degraded` until
  manual visual QA, native backdrop/shadow decisions, focused anchoring, fullscreen/UAC, and multi-monitor behavior
  are complete.

## Phase 10aw Windows DWM Backdrop Probe Disabled

Phase 10aw does not change Windows overlay capability levels:

- The Windows overlay briefly tried `DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_TRANSIENTWINDOW`.
- Manual QA showed backdrop/desktop content outside the rounded overlay boundary, likely because DWM backdrop
  composition remains rectangular while the current overlay uses `WS_EX_LAYERED` / `UpdateLayeredWindow` with
  rounded per-pixel content.
- The DWM backdrop route is disabled. The existing Direct2D per-pixel translucent surface remains the current
  renderer path.
- `overlay.material` stays `degraded/translucent_fallback_only`; future blur work should evaluate a composition
  route that owns blur and rounded clipping together.

## Phase 10ay Windows Direct2D Per-Pixel Shadow Polish

Phase 10ay does not change Windows overlay capability levels:

- The Direct2D renderer draws a renderer-owned soft shadow inside the existing premultiplied-alpha
  `UpdateLayeredWindow` surface, using an expanded transparent surface and inset panel rect.
- This avoids the DWM backdrop route that polluted pixels outside the rounded overlay boundary.
- GDI remains a fallback without shadow.
- `overlay.material` stays `degraded/translucent_fallback_only`; this phase is surface polish, not blur,
  Acrylic/Mica, Liquid Glass parity, or final visual QA.

## Phase 10az Windows Foreground Monitor Work Area

Phase 10az does not change Windows overlay capability levels:

- Windows overlay placement now selects the foreground window's nearest monitor work area before falling back to
  `SPI_GETWORKAREA`.
- This improves screen anchoring for common multi-monitor setups and mixed taskbar/work-area layouts.
- It does not implement focused-window anchoring, caret anchoring, or foreground window geometry following.
- `overlay.window_anchor` remains `degraded/screen_anchor_only` until focused anchoring and multi-monitor visual QA
  are complete.

## Phase 10ah Windows Audio Capture Diagnostics

Windows `audio.capture` 在 Phase 10ah 只表达 cpal/WASAPI 诊断探针存在：

- `audio.capture`：`partial`，backend `cpal_wasapi`，reason `diagnostic_probe_only`。
- `shuo doctor` 可以打印默认输入设备 config 和 input device count。
- 该探针不启动录音流、不写 retained audio、不触发 ASR、overlay、hotkey、clipboard 或 paste。
- 在用户完成真实麦克风录音、权限/隐私设置、采样格式转换、静音/噪声底和持续采集测试前，
  Windows `audio.capture` 不能升级到 `degraded` 或 `available`。

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
