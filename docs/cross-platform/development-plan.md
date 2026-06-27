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
- 不引入配置编辑器。

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

## Phase 9: GUI PoC Archived

GUI/Tauri PoC 已从当前跨平台 runtime 分支移出，归档在 `feat/gui-poc-archive`。
当前分支不携带 GUI workspace、静态 frontend、GUI client helper 或 library split 代码。

当前阶段只保留 runtime 约束：daemon、TUI、overlay、hotkey、audio、IPC、service 等核心路径
不得引入 WebView 或桌面 GUI runtime。未来重新开发 GUI 时，应从归档分支取回需要的设计材料，
基于当时最新 `main` 重新评估产品范围和依赖边界。

## Phase 10: First Non-macOS Core Backend

目标：在不依赖完整 overlay 或 packaged app 的前提下，让核心能力在第一个非 macOS 平台可运行。

当前顺序：

1. 保留已完成的 Linux compile/capability/service dry-run 基线，作为 Unix 侧回归保护。
2. Windows-first core：path/config/state、IPC endpoint security、single instance runtime、audio、
   overlay、hotkey、clipboard/paste。
3. Windows 本地开发机负责 build/runtime test；GitHub 只负责代码同步，不用 GitHub Actions 产物。
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
- Windows should not link ONNX Runtime during the current Windows-first core runtime phase. The Windows target
  also uses the local Silero unavailable stub and does not depend on `voice_activity_detector` until ONNX
  Runtime provisioning is designed for the installed MSVC toolchain. This keeps build/test and IPC runtime
  smoke unblocked without claiming Windows VAD or audio support.
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
- Add `docs/cross-platform/app-data.md` as the shared CLI/daemon/TUI/packaged app data ownership model.
- This phase is docs-only. It must not change Windows behavior or promote any Windows capability.

Phase 10n Windows Runtime Validation Checklist:

- Add `docs/cross-platform/windows-runtime-validation.md`, a Windows validation checklist the user can run
  directly on Windows.
- Include exact commands, expected observable behavior, and where to paste command output.
- Scope the first checklist to version/doctor/config paths, state/history/log path creation, Named Pipe daemon
  status, single-instance smoke, service dry-run status, and Explorer open/reveal.
- Do not include audio, overlay, hotkey, or paste in the first checklist until a testable artifact exists.

Phase 10o Windows Path/Config/State Backend:

- Add an `AppPaths` product path facade in `src/paths.rs`.
- Route config path helpers and `StateDirs` through `AppPaths`.
- Add a Windows backend using known-folder APIs first: config under `%APPDATA%\Shuohua`,
  state/history/audio/logs/traces/cache under `%LOCALAPPDATA%\Shuohua`.
- Keep macOS/Linux terminal-friendly config behavior: XDG or `~/.config/shuohua`; do not migrate macOS paths.
- Add tests that protect Windows from using Unix dotfile/XDG/HOME paths and protect package-private data from
  becoming the product data root.
- This phase must not change config schema, history schema, IPC protocol, or directory creation timing.
- Passing `make check-windows` proves compile/cfg boundaries only. Product path behavior still needs the
  Windows runtime checklist.

Phase 10p Windows Local Development Setup:

- Add `docs/cross-platform/windows-local-dev.md`, documenting Git sync, MSVC/Rust setup, local build/test
  commands, and how to report runtime smoke results back to the macOS session.
- Do not add a `windows-latest` CI artifact job in this phase. GitHub Actions is too slow for the current
  Windows-first runtime loop.
- Keep local build separate from runtime validation. A local Windows build proves the binary exists on Windows;
  desktop capabilities still need the runtime checklist.
- This phase should run after the Windows path backend so the first manual builds create data in final
  locations.

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
- Packaged app product work remains out of scope during these subphases.

Phase 10ah Windows Audio Capture Diagnostics:

- Add a `platform::audio_capture` diagnostic facade over `cpal` before attempting full recording.
- `shuo doctor` may print the selected backend, default input device summary, and input device count. This is
  safe to run in the normal Windows build/test loop because it does not start a recording stream, write retained
  audio, call ASR, or trigger paste/overlay/hotkey behavior.
- Windows `audio.capture` may move from unsupported to `partial/cpal_wasapi/diagnostic_probe_only`, but this
  only means device enumeration/default config diagnostics exist.
- Do not promote beyond `partial` until a user manually validates microphone permission behavior, actual
  recording, sample format conversion, silence/noise floor, and sustained capture on Windows.

Phase 10ai Platform-Specific Profile Routes:

- Replace the macOS-shaped `[profile] profile_name = ["bundle.id"]` route table with
  `[profile.routes.<profile>.<platform>]` matcher tables.
- Keep profile files, ASR provider configs, post chains, prompts, and hotwords shared across platforms; only
  app identity matchers are platform-specific.
- Do not implement Windows active app lookup in this phase. Windows/Linux matcher schema can exist while their
  runtime identity backend still returns no app identity and falls back to `profile.default`.
- The old top-level profile array route shape is rejected because there are no external users to migrate.

Phase 10aj Windows Active App Identity Diagnostics:

- Add the first Windows `platform::desktop::frontmost_app()` backend using foreground-window owner process
  metadata.
- Scope the first step to `exe_name`; keep `app_user_model_id` as schema/model reserve until a separate AUMID
  lookup is designed and runtime-tested.
- Expose a privacy-safe `shuo doctor` diagnostic line for runtime smoke. Do not print full process paths.
- Allow Windows profile routes to match `profile.routes.<profile>.windows.exe_name`, but keep
  `desktop.active_app` at `partial/foreground_window_process_exe/exe_name_only`.
- Do not start audio, overlay, hotkey, clipboard, paste, or full recording validation in this phase.

Phase 10ak Windows Profile Route Diagnostics:

- Add a read-only `shuo doctor` diagnostic for the current active app identity -> profile route decision.
- Reuse the same `ProfileRouteCfg::matching_profiles` and `AppIdentity::current_from_app_context` path as
  daemon session start; do not duplicate route semantics.
- Print default fallback, single route match, or duplicate-match error clearly enough to debug config before
  audio/hotkey runtime exists.
- Do not start recording, touch provider runtime, or trigger overlay/hotkey/clipboard/paste.

Phase 10al Windows Clipboard Write Backend:

- Implement only Windows Unicode clipboard writes behind the existing `platform::clipboard` facade.
- Use Win32 `OpenClipboard` / `EmptyClipboard` / `SetClipboardData(CF_UNICODETEXT)` with movable global memory.
- Keep paste injection unsupported; do not call `SendInput`, do not implement hotkey, overlay, audio, or full
  record -> paste flow in this phase.
- Capability may move from unsupported to partial/write-only after build and same-session runtime smoke, but must
  not imply paste injection or target-app parity.

Phase 10am Windows Paste Injection Backend:

- Implement only `platform::autotype::paste()` on Windows as a `SendInput` Ctrl+V sequence.
- Keep clipboard write and paste injection as separate capabilities; paste depends on foreground focus and target
  app behavior.
- Add an explicit ignored runtime smoke because it sends real keyboard input to the foreground app.
- Do not implement global hotkey, overlay, audio, or full record -> ASR -> post -> paste flow in this phase.
- Capability may move to partial after same-session smoke, but must not imply target-app or UAC/elevation parity.

Phase 10an Windows Low-Level Hotkey Backend:

- Implement only the Windows hotkey provider backend with `WH_KEYBOARD_LL` on a dedicated OS thread.
- Reuse the existing `RawEvent` wire format and `Suppressor` decision path so daemon hotkey tracking stays shared.
- Map Windows virtual keys to the existing platform-neutral `Key` model; unknown keys must still flow as
  `Key::Unknown`.
- Add an explicit ignored runtime smoke because it installs a global keyboard hook and observes real key events.
- Do not start audio, overlay, clipboard/paste, provider runtime, or full record -> paste flow in this phase.
- Capability may move to partial/runtime-smoke only after a same-session hook smoke, but suppression and target-app
  parity still require manual foreground-app validation.

Phase 10ao Windows Minimal Overlay Backend:

- Replace the Windows overlay skeleton with a minimal native Win32 renderer that creates one dedicated
  no-activate/topmost/toolwindow/layered window and consumes the existing `OverlayCmd` channel.
- Use only Win32/GDI for the first backend; do not introduce Tauri, WebView, Direct2D, Skia, wgpu, or a large
  renderer stack in this phase.
- Reuse shared `OverlayModel` and layout helpers; the Windows backend should only own window creation, message
  pump, show/hide, and basic translucent text drawing.
- Add a runtime smoke that can create/show/hide/quit the renderer without audio or full recording.
- Capability may move to partial after smoke, but input passthrough, focused-window anchoring, advanced material,
  multi-monitor behavior, and real visual quality remain manual validation gates.

Phase 10ap Windows Overlay DPI And Font Baseline:

- Fix Windows overlay sizing and placement before visual polish: use monitor work area and DPI scale instead of
  raw primary-screen pixels and fixed unscaled layout constants.
- Keep the renderer native Win32. Do not introduce Direct2D/DirectWrite, Acrylic/Mica, bundled fonts, or
  advanced animation in this phase.
- Use the platform UI font path first. macOS already uses `NSFont::systemFontOfSize` /
  `boldSystemFontOfSize`; it does not require JetBrains Mono or bundled SF Pro. Windows should likewise prefer
  system UI fonts for prose text.
- Do not bundle SF Pro. If a monospace or branded fallback is needed later, treat JetBrains or another OFL font as
  an optional packaged fallback, not a hard runtime requirement.
- Capability remains partial/degraded until high-DPI, multi-monitor, and final visual quality are manually
  validated.

Phase 10aq Windows Overlay Rounded GDI Baseline:

- Close the biggest parity gap visible after Phase 10ap without changing renderer architecture: apply the shared
  `overlay.surface.corner_radius` to the Win32 window shape and use shared `background_alpha` for layered-window
  opacity.
- Keep this as a GDI baseline: use ClearType font quality for `CreateFontW`, but do not introduce Direct2D,
  DirectWrite, Acrylic/Mica, DWM shadow experiments, or animation in this phase.
- This phase may improve obvious rectangle/text roughness, but it is not a final visual-quality claim. If text
  still looks softer than Windows system UI, the next renderer-quality step should be DirectWrite/Direct2D.
- Capability remains partial/degraded until user-visible QA covers real foreground apps, multi-monitor,
  fullscreen/UAC, and final material/text quality.

Phase 10ar Windows Direct2D/DirectWrite Renderer Foundation:

- Move Windows overlay text and rounded-surface drawing from GDI to Direct2D + DirectWrite while keeping the
  existing Win32 popup/no-activate/topmost/click-through window shell.
- Keep Direct2D/DirectWrite isolated in a Windows-only renderer module. Do not expose COM/D2D/DWrite types to
  shared overlay model/layout code or daemon business layers.
- Use the `windows` crate typed COM bindings for Direct2D/DirectWrite; keep `windows-sys` for existing lightweight
  Win32/lifecycle/IPC backends.
- First implementation uses `ID2D1HwndRenderTarget` and DirectWrite text formats. Do not introduce
  DirectComposition, D3D/DXGI device chains, `UpdateLayeredWindow` per-pixel surfaces, Acrylic/Mica, shadows, or
  animation until the text-quality foundation is stable.
- Keep GDI fallback available if Direct2D/DirectWrite initialization or painting fails, so overlay remains usable on
  machines with graphics stack issues.
- Capability remains partial/degraded until user-visible QA confirms text clarity, real foreground app behavior,
  fullscreen/UAC, multi-monitor behavior, and material/shadow decisions.

## 持续维护

- 每完成一个 phase，更新 `overview.md` 的阶段状态。
- 发现文档假设错误，先改文档，再改实现。
- 不把 PoC 临时日志放进长期文档；只记录结论、风险和决策。
