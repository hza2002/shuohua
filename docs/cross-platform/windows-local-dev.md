# Windows Local Development

Windows runtime work is developed and tested on a Windows machine, synced through Git. GitHub CI remains useful
for macOS regression, but it is not the Windows build path because Windows CI is too slow for this phase.

Use this document when setting up or refreshing the Windows development machine.

## Repository Sync

On macOS:

```sh
git status --short --branch -uall
git push origin feat/cross-platform-design
```

On Windows PowerShell:

```powershell
git clone <repo-url> shuohua
cd shuohua
git checkout feat/cross-platform-design
git pull --ff-only
```

If the repository already exists:

```powershell
cd shuohua
git status --short --branch -uall
git pull --ff-only
```

Do not edit the same files on macOS and Windows at the same time. Commit or stash before switching machines.

## Toolchain

Install:

- Git for Windows.
- Visual Studio Build Tools with the MSVC C++ build tools and Windows SDK.
- Rust stable with the MSVC toolchain.

PowerShell checks:

```powershell
git --version
rustup --version
rustc -Vv
cargo -V
rustup default stable-x86_64-pc-windows-msvc
rustup target add x86_64-pc-windows-msvc
```

Expected:

- `rustc -Vv` host is `x86_64-pc-windows-msvc`.
- `cargo` commands run in a normal non-elevated PowerShell unless a specific elevated-boundary test says
  otherwise.

## Build And Test

First Windows local build:

```powershell
cargo fmt --check
cargo test --target x86_64-pc-windows-msvc
cargo build --target x86_64-pc-windows-msvc
```

Binary path:

```powershell
.\target\x86_64-pc-windows-msvc\debug\shuo.exe --version
.\target\x86_64-pc-windows-msvc\debug\shuo.exe doctor
```

For quick iteration after the target is already defaulted to MSVC, plain commands are acceptable:

```powershell
cargo test
cargo build
.\target\debug\shuo.exe doctor
```

Record which command produced the binary when reporting test results.

## Runtime Smoke Entry

After build succeeds, run [windows-runtime-validation.md](windows-runtime-validation.md) using the locally built
binary.

Use the exact binary path in the checklist commands. Example:

```powershell
$Shuo = ".\target\x86_64-pc-windows-msvc\debug\shuo.exe"
& $Shuo --version
& $Shuo doctor
```

Do not validate audio, overlay, hotkey, clipboard, paste, or end-to-end recording until those backend phases
provide their own checklist.

## Result Handoff

When reporting Windows results back to the macOS development session, include:

- Commit hash tested: `git rev-parse --short HEAD`.
- `rustc -Vv`.
- Whether PowerShell was elevated.
- Build command used.
- Full output or screenshots for failing commands.
- Runtime checklist section where the failure happened.

If Windows local changes are made:

```powershell
git status --short --branch -uall
git diff
git add <files>
git commit -m "<message>"
git push origin feat/cross-platform-design
```

Then macOS should pull with:

```sh
git pull --ff-only
```
