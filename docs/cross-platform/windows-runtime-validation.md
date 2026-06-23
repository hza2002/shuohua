# Windows Runtime Validation

This checklist is for the first Windows smoke test. It validates the bottom of the runtime stack only:
artifact identity, product data paths, daemon/client IPC, single instance behavior, service dry-run status, and
Explorer open/reveal.

Do not use this checklist to claim audio, overlay, hotkey, clipboard, paste, or end-to-end recording support.
Those capabilities need later backend work and separate Windows runtime tests.

## Environment Record

Before running commands, record:

```powershell
systeminfo | findstr /B /C:"OS Name" /C:"OS Version" /C:"System Type"
$PSVersionTable.PSVersion
whoami
```

Also record:

- Windows version: Windows 11 or Windows 10.
- Machine type: real machine or VM.
- Whether the shell is elevated.
- Artifact source: local build, CI artifact, or copied debug build.

## Artifact Setup

Open PowerShell in the directory containing `shuo.exe`.

```powershell
.\shuo.exe --version
.\shuo.exe doctor
```

Expected:

- `--version` prints a version and exits.
- `doctor` runs without crashing.
- Windows capabilities may still report `partial`, `unsupported`, or `runtime_not_verified`.

## Product Data Paths

Run:

```powershell
Write-Host "APPDATA=$env:APPDATA"
Write-Host "LOCALAPPDATA=$env:LOCALAPPDATA"
Test-Path "$env:APPDATA\Shuohua"
Test-Path "$env:LOCALAPPDATA\Shuohua"
```

Expected after path backend work:

- Config paths resolve under `%APPDATA%\Shuohua`.
- State/history/audio/logs/traces resolve under `%LOCALAPPDATA%\Shuohua`.
- Product data does not resolve under the executable directory, `Program Files`, `%USERPROFILE%\Shuohua`, or
  package-private app data.

If the directories do not exist before config/history activity, record that. Directory creation timing should
be explicit, not guessed.

## Daemon And IPC Smoke

Open PowerShell window A:

```powershell
.\shuo.exe daemon
```

Open PowerShell window B:

```powershell
.\shuo.exe doctor
.\shuo.exe service status
```

Expected:

- The daemon starts or reports a clear error.
- `doctor` and `service status` do not hang.
- IPC status is visible when the daemon is running.
- Current Windows Named Pipe transport is not runtime-accepted until endpoint scoping and DACL behavior are
  implemented and tested.

## Single Instance Smoke

Keep window A running the daemon. Open PowerShell window C:

```powershell
.\shuo.exe daemon
```

Expected:

- The second daemon exits or reports that an instance already exists.
- It must not create an independent usable runtime endpoint.
- If behavior differs between elevated and non-elevated shells, record both cases.

## Service Dry-Run Status

Run:

```powershell
.\shuo.exe service status
```

Expected:

- Status prints daemon availability plus Windows user-session dry-run strategy.
- It must not install a Task Scheduler task, write registry keys, call SCM service APIs, or require elevation.

## Explorer Open/Reveal

Use the TUI or CLI action that opens/reveals config/audio paths when available. For the first manual smoke, also
record direct path behavior:

```powershell
explorer.exe "$env:APPDATA\Shuohua"
explorer.exe "$env:LOCALAPPDATA\Shuohua"
```

Expected:

- Explorer opens the intended directory if it exists.
- Missing directories produce clear behavior that can be documented before promotion.

## Stop Points

Stop and report results after this checklist if any of the following happens:

- `doctor` crashes or hangs.
- Daemon start does not produce a clear running/error state.
- A second daemon can run independently.
- Product data resolves to package-private data or the executable directory.
- `service status` performs real install/start/stop work instead of dry-run status.

Do not continue to audio, overlay, hotkey, clipboard, paste, or end-to-end recording validation until the
corresponding backend phase provides a new checklist.
