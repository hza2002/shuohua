# Windows Development Design

## Scope

Windows is the next primary cross-platform target. The goal is a normal per-user desktop application that
can record microphone audio, receive global hotkeys, show a native overlay, paste text into the foreground
application, and expose the same CLI/TUI/GUI client contract as macOS.

This document is the implementation baseline for Windows-specific development. If implementation or runtime
testing proves a better route, update this document before changing the code.

Current status:

- `make check-windows` proves `x86_64-pc-windows-msvc` cfg/type boundaries only.
- Named Pipe IPC, named mutex lifecycle primitives, path open/reveal, service dry-run status, and capability
  truthfulness have compile-checked skeletons.
- No Windows runtime behavior is verified yet. Do not mark Windows capabilities `available` until tested on
  Windows.

Primary baseline:

- Windows 11, per-user desktop install, non-elevated.
- Windows 10 remains supported where core APIs exist, with visual/material fallback accepted.
- The Windows machine is a runtime validation target. macOS remains the preferred edit/test/cross-check host
  until Windows-specific runtime tests require the real desktop.

## Non-Negotiable Direction

- Windows support must not introduce Tauri/WebView into daemon hot paths.
- The daemon must run in the interactive user session, not as a classic Windows Service.
- macOS behavior, JSON-line IPC protocol, history schema, and config schema remain stable unless a migration is
  documented.
- Windows runtime validation must happen on Windows hardware or a Windows VM. macOS cross-checks are not
  runtime evidence.
- Windows features should follow Windows user-profile, security, startup, and desktop-interaction conventions
  first. Unix compatibility is not a reason to put data in dotfiles, run desktop work as a service, or rely on
  global IPC names.

## Process Model

Windows keeps the same daemon + clients model:

```text
shuo daemon      user-session process: audio / hotkey / overlay / clipboard / history / IPC
shuo CLI/TUI     on-demand client over Named Pipe
Shuo GUI         future Tauri client, not part of daemon hot path
```

The daemon owns desktop integration because hotkey, clipboard, text injection, overlay windows, and microphone
capture are user-session capabilities. A Windows Service is not the default route because services run outside
the normal interactive desktop model and are a poor fit for clipboard/text injection and overlay rendering.

Startup should be user scoped:

- Preferred first runtime route: explicit `shuo daemon` launched by the user or a development script.
- Later install route: Task Scheduler logon task or Startup App registration for the current user.
- Avoid elevated install and machine-wide service management unless a future requirement proves it necessary.

## File Layout

Windows file layout follows the shared product data ownership model in [app-data.md](app-data.md). The same
product config/history/audio/log roots are shared by CLI, daemon, TUI, GUI, and tray. Package-private app data
is reserved for GUI/runtime state and must not become a second product data truth source.

Windows must use Windows per-user app data locations, not Unix-style home dotfiles.

Recommended mapping:

| Data | Windows location | Notes |
|---|---|---|
| User-editable config | `%APPDATA%\Shuohua\` | `config.toml`, `profile\`, `asr\`, `post\`; suitable for user settings and possible profile sync. |
| Local state | `%LOCALAPPDATA%\Shuohua\` | Runtime state that should not roam. |
| History | `%LOCALAPPDATA%\Shuohua\history\` | Preserve existing JSONL schema. |
| Retained audio | `%LOCALAPPDATA%\Shuohua\audio\` | Large/local data; do not roam. |
| Logs | `%LOCALAPPDATA%\Shuohua\logs\` | Local diagnostics only. |
| Traces | `%LOCALAPPDATA%\Shuohua\traces\` | Dev/diagnostic only; never sensitive text by default. |
| Cache/temp | `%LOCALAPPDATA%\Shuohua\cache\` or `%TEMP%` | Rebuildable data only. |
| Per-user binary install | `%LOCALAPPDATA%\Programs\Shuohua\` | Future installer path; no admin requirement. |

Implementation notes:

- `src/paths.rs` currently uses XDG/HOME behavior. Windows support must add a Windows backend before runtime
  validation.
- Windows unpackaged desktop builds should resolve known folders through Windows APIs when possible, with
  `%APPDATA%` / `%LOCALAPPDATA%` environment fallback only as a development fallback.
- Packaged-app `ApplicationData` or MSIX package-local data is app-private by default. It may store GUI window
  state, WebView cache, onboarding state, tray preference, or updater/package runtime state, but it must not
  silently replace the shared product data root used by CLI and daemon.
- If a future packaged app cannot access the shared product data root because of sandbox/store constraints,
  add an explicit migration/import/export design before changing roots.
- Do not store transcripts, audio, or logs in `Program Files`, the executable directory, or system-wide
  locations.
- Do not create app data directly under `%USERPROFILE%`; Microsoft documents the profile root as inappropriate
  for normal application folders.
- Config backup/sync behavior is intentionally limited to the user-editable config directory. History, retained
  audio, traces, and logs stay local because they may be large or sensitive.
- Future path implementation should converge behind an `AppPaths`-style facade so business modules do not read
  `%APPDATA%`, `%LOCALAPPDATA%`, package paths, or install paths directly.

Initial path validation checklist:

```powershell
.\shuo.exe doctor
.\shuo.exe config path
.\shuo.exe history list
Test-Path "$env:APPDATA\Shuohua"
Test-Path "$env:LOCALAPPDATA\Shuohua"
```

## IPC

Windows IPC uses Named Pipes and keeps the same JSON-line protocol.

Default endpoint design should move from the current compile skeleton `\\.\pipe\shuohua` to a user-session
scoped pipe before runtime-ready status:

```text
\\.\pipe\shuohua-<logon-sid-or-session-scoped-hash>
```

Security requirements:

- The pipe must not be globally writable/readable by unrelated users or sessions.
- Before marking IPC available, create a security descriptor/DACL that restricts access to the current user or
  logon SID.
- Do not rely on the default Named Pipe security descriptor for final runtime behavior.
- Use the logon SID when practical so elevated/non-elevated processes in the same logon session can be reasoned
  about explicitly and unrelated terminal sessions do not share the endpoint.
- Avoid `GENERIC_WRITE` for final access masks where narrower rights are sufficient because Microsoft documents
  that generic pipe write rights include pipe-instance creation rights.
- Server accept flow should keep the existing pattern: once one server instance connects, prepare the next
  instance before handing the connected stream to protocol handling.
- Client connect may retry `ERROR_PIPE_BUSY`; it must not silently start the daemon until smart fallback is
  explicitly implemented.

Validation gates:

- One daemon + one CLI can exchange `DaemonStatus`.
- Multiple sequential clients can connect.
- Pipe busy retry behaves predictably.
- A second daemon fails single-instance / first-pipe-instance checks.
- Cross-user or elevated/non-elevated behavior is documented from real Windows tests.

## Single Instance And Process Probe

The Windows daemon guard should be user-session scoped:

```text
Local\shuohua-<logon-sid-or-session-scoped-hash>
```

Current `Local\shuohua-daemon` is a compile skeleton and must be hardened before runtime-ready status.

Rules:

- Use a named mutex for daemon startup exclusion.
- The mutex name must use the same user/session identity material as the pipe name.
- Treat abandoned mutex behavior explicitly during runtime testing; document whether it is recovered or treated
  as a warning.
- Use `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, ...)` for process probe only as a liveness hint.
- Do not build correctness on PID liveness alone; process reuse remains a risk.
- A second daemon should fail before opening a second runtime endpoint. The final guard is named mutex plus pipe
  first-instance behavior, not one primitive alone.

## Service And Startup

Windows service management means user-session lifecycle, not SCM service by default.

Phased route:

1. `shuo service status`: dry-run and daemon IPC status only.
2. `shuo service install --dry-run` or equivalent preview: show intended user logon task/startup registration.
3. Runtime-tested Task Scheduler logon trigger for current user.
4. Optional Startup Apps registration if it provides a better user experience.

Hard boundaries:

- No SCM Windows Service for the desktop daemon unless a future split creates a non-desktop helper.
- No admin requirement for normal install/start.
- No registry writes until the exact key, ownership, rollback, and uninstall behavior are documented.
- `stop` should prefer IPC shutdown and bounded wait, mirroring macOS semantics.
- Do not use interactive services. Modern Windows services cannot directly interact with the user desktop, and
  the desktop-facing daemon needs microphone, overlay, clipboard, text injection, and foreground-window context.
- Do not use `schtasks`/COM registration until install/uninstall idempotency, task name, task folder, trigger,
  principal, working directory, and rollback behavior are documented.

Future scheduled task baseline:

| Field | Baseline |
|---|---|
| Task folder | `\Shuohua\` |
| Task name | `Shuohua Daemon` |
| Trigger | current user logon |
| Principal | current user, least privilege, no elevation prompt |
| Action | installed `shuo.exe daemon` |
| Working directory | install directory |
| Stop policy | IPC stop first, then task stop only if explicitly implemented |

## Audio Capture

First implementation route:

- Continue using `cpal` if it provides acceptable WASAPI capture behavior.
- Treat Windows audio capability as `partial/runtime_not_verified` until microphone device enumeration,
  permission behavior, sample format conversion, silence/noise floor, and sustained recording are tested.
- If `cpal` is insufficient, evaluate a Windows-specific WASAPI backend behind `platform::audio`.

Runtime validation must cover:

- External microphone detection.
- Default input device changes while daemon is running.
- App microphone privacy settings.
- 16 kHz mono pipeline invariants after resampling.
- Retained audio behavior when `record_audio` is compact/lossless/off.

Audio conversion remains unsupported on Windows until a converter backend is chosen and runtime-tested.

Stop point for user intervention:

- After `shuo.exe doctor` can show a Windows audio backend and device summary.
- Before promoting `audio.capture` beyond `partial`, the user must run a real microphone recording test on
  Windows because macOS cannot validate WASAPI device selection, privacy prompts, or capture stability.

## Hotkey

Windows hotkey behavior needs parity with macOS press/release tracking and suppression.

Candidate routes:

- `WH_KEYBOARD_LL` low-level keyboard hook: preferred PoC for hold-to-record and suppression parity.
- `RegisterHotKey`: useful for simple global hotkey notification, but insufficient by itself for press/release
  tracking and suppression semantics.
- Raw Input: useful for device-level input experiments, but not the first choice for suppressing keystrokes.

Implementation constraints:

- The hook/message loop must live on a dedicated OS thread.
- The callback must not perform async business logic.
- Down/up pairing must be preserved, especially for suppressed keys.
- The hook must degrade cleanly when integrity-level/UAC boundaries prevent observing or injecting into an
  elevated foreground app.
- Runtime tests must cover stuck modifier prevention, IME interaction, focus changes, remote desktop, and
  conflicting system/application hotkeys.

Stop point for user intervention:

- After a compile-checked hook backend exists and can print key down/up diagnostics on Windows.
- Suppression and hold-to-record parity require manual testing in real foreground applications.

## Clipboard And Text Injection

First implementation route:

- Clipboard write: Win32 clipboard APIs with Unicode text.
- Paste injection: `SendInput` for Ctrl+V into the foreground application.
- Active app lookup: foreground window + process metadata, with privacy-safe display in history/TUI.

Rules:

- Match current macOS user-visible semantics first; do not introduce clipboard restore behavior until it is
  designed and tested.
- If paste injection fails, history should still record the transcription status accurately.
- Text injection must be tested across Notepad, browser text fields, Office/Teams-style apps if available, and
  terminal/editor windows.
- Foreground restrictions and UAC/elevation boundaries must be documented from runtime tests.
- Clipboard access is foreground-desktop state. Keep it in the daemon desktop facade; do not move it into GUI
  code or service code.

## Overlay

Windows overlay must be native Win32, not Tauri/WebView.

First PoC route:

- Dedicated overlay thread with a Win32 message loop.
- Borderless top-level popup created with `CreateWindowEx`.
- Extended styles: start with `WS_EX_TOPMOST`, `WS_EX_TOOLWINDOW`, `WS_EX_NOACTIVATE`, and `WS_EX_LAYERED`.
- Position with `SetWindowPos`.
- Material baseline: `solid` or `translucent`; advanced DWM/Mica/Acrylic is optional and must degrade cleanly.
- Text rendering route may be Direct2D, softbuffer, Skia, or another small native renderer after PoC. Do not
  pick a large rendering stack until a minimal visible overlay works.

Validation gates:

- Shows/hides without stealing focus.
- Stays above normal foreground apps.
- Text is readable on light/dark backgrounds.
- Click-through/input passthrough behavior is measured separately from no-activate behavior.
- Multi-monitor and DPI scaling are tested.
- Windows 11 and Windows 10 fallback behavior are recorded separately.

Stop point for user intervention:

- After the minimal Win32 overlay can be launched manually and accepts show/hide/status commands.
- Visual behavior, focus behavior, monitor placement, and DPI cannot be accepted from macOS cross-checks.

## GUI

GUI product work remains frozen. Windows work should not polish the current placeholder GUI.

Allowed:

- Keep shared client API compatible with Windows Named Pipe.
- Add Windows build/bundle notes when CI artifacts exist.

Not allowed in the Windows core phase:

- Move daemon logic into Tauri.
- Add GUI reconnect/service management as a substitute for daemon runtime validation.
- Add WebView to overlay or hotkey paths.

## Build And Artifact Strategy

Development split:

- macOS: edit code, run tests, run `make check-windows`.
- Windows: build and runtime-test artifacts.
- CI: use `windows-latest` to produce `shuo.exe` artifacts before asking for repeated manual runtime testing.

Mac commands:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
make check-windows
make check-linux-cross
```

Windows local commands:

```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --target x86_64-pc-windows-msvc
cargo test --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\debug\shuo.exe doctor
.\target\x86_64-pc-windows-msvc\debug\shuo.exe service status
```

Artifact strategy:

- Short term: build on the Windows machine or CI. macOS `cargo check --target x86_64-pc-windows-msvc` is only a
  compile boundary check, not an artifact packaging route.
- Do not ask the user to repeatedly rebuild on Windows after every small code change once CI artifact upload is
  available.
- A Windows artifact is testable only after `shuo.exe`, any required DLLs, and the expected config/runtime path
  behavior are captured in the validation notes.

Preferred artifact route before repeated manual testing:

- Add CI on `windows-latest` to build `shuo.exe` and upload an artifact.
- The Windows machine should primarily test artifacts, not be the main development environment.
- Do not claim a Windows release until artifact build, runtime smoke test, and manual desktop checks pass.

## Runtime Validation Order

Do not start with overlay. Validate the runtime stack bottom-up:

1. `shuo.exe version`, `doctor`, config path discovery.
2. State/history/log path creation under `%LOCALAPPDATA%\Shuohua`.
3. Named Pipe daemon/client `DaemonStatus`.
4. Single-instance mutex.
5. `service status` dry-run.
6. Path open/reveal via Explorer.
7. Audio capture with external microphone.
8. TUI status/history.
9. Overlay visible/no-activate/topmost smoke.
10. Hotkey press/release/suppression.
11. Clipboard write and paste injection.
12. End-to-end record -> ASR -> post -> clipboard/paste -> history.

User intervention points:

| Phase | Needs user? | Why |
|---|---:|---|
| Path/config/state backend | No, until Windows artifact exists | Mostly compile and unit-testable. |
| Named Pipe + mutex runtime smoke | Yes | Needs Windows process/session behavior. |
| Service dry-run/install preview | Yes before real install | User must approve startup registration. |
| Audio capture | Yes | Requires real microphone and Windows privacy state. |
| Overlay | Yes | Visual/focus/DPI behavior is runtime-only. |
| Hotkey suppression | Yes | Needs real foreground apps and keyboard state. |
| Clipboard/paste | Yes | Needs target apps and UAC/elevation boundaries. |
| CI artifact upload | No | Agent can implement once repository policy is clear. |

## Capability Promotion Rules

Capability status can only move upward with evidence:

- `unsupported` -> `partial`: compile backend exists or dry-run path exists.
- `partial` -> `degraded`: runtime works with known limitations.
- `degraded` -> `available`: runtime works in the documented baseline environment with acceptable fallback.

For Windows, every promotion must record:

- Windows version.
- Machine type.
- Input device/microphone.
- Foreground apps tested.
- Relevant permission/privacy settings.
- Exact command output or screenshot/video note for visual behavior.

## Initial Windows Phases

Recommended next phases:

1. Windows runtime validation document and command checklist.
2. Windows path/config/state directory backend.
3. Windows CI artifact build for `shuo.exe`.
4. Windows Named Pipe endpoint scoping/security hardening.
5. Windows Named Pipe + mutex runtime smoke.
6. Windows audio capture smoke with `cpal`/WASAPI.
7. Windows overlay minimal visible PoC.
8. Windows hotkey low-level hook PoC.
9. Windows clipboard + paste injection PoC.

Stop for user testing after phases 3, 4, 5, 6, and 7. Those cannot be validated on macOS.

Do not start with GUI polish. The GUI remains a future client surface; the Windows-first core path is CLI/TUI,
daemon, native overlay, hotkey, audio, clipboard, and paste.

## References

- Microsoft app data guidance:
  <https://learn.microsoft.com/en-us/windows/apps/develop/data/store-and-retrieve-app-data>
- Microsoft Named Pipe security and access rights:
  <https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights>
- Microsoft interactive services guidance:
  <https://learn.microsoft.com/en-us/windows/win32/services/interactive-services>
- Microsoft Task Scheduler logon trigger example:
  <https://learn.microsoft.com/en-us/windows/win32/taskschd/logon-trigger-example--scripting->
- Microsoft keyboard input overview:
  <https://learn.microsoft.com/en-us/windows/win32/inputdev/about-keyboard-input>
- Microsoft `RegisterHotKey`:
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey>
- Microsoft Raw Input:
  <https://learn.microsoft.com/en-us/windows/win32/inputdev/using-raw-input>
- Microsoft `SendInput`:
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput>
- Microsoft clipboard overview:
  <https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard>
- Microsoft WASAPI overview:
  <https://learn.microsoft.com/en-us/windows/win32/coreaudio/wasapi>
- Microsoft `CreateWindowEx`:
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexa>
- Microsoft extended window styles:
  <https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles>
- Microsoft Mica material:
  <https://learn.microsoft.com/en-us/windows/apps/design/style/mica>
- Microsoft `DwmSetWindowAttribute`:
  <https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmsetwindowattribute>
