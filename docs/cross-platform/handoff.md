# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近 commit

HEAD: `docs: record windows pipe client access mask limit`（Phase 10x 提交；精确 hash 以 `git log -1` 为准）。

Previous commit: `fix: share windows ipc scope across elevation` (`e7d8965`).

当前分支已 rebase 到 `v0.2.0` / `release: v0.2.0` 基底（commit `7fff199`）。

## 当前 phase

GUI PoC 冻结，当前主线切到 Windows-first core runtime。
Phase 10x Windows Named Pipe client access-mask audit 已完成：

- 复查 Tokio 1.52 `tokio::net::windows::named_pipe::ClientOptions`：当前公开 client `read`/`write`
  选项映射到 Windows `GENERIC_READ` / `GENERIC_WRITE`，不能传入更窄的 desired access mask。
- 因此当前 Windows IPC hardening 范围仍是 endpoint scope、server-side DACL、mutex security descriptor、
  elevation split 修复和 runtime smoke；client access mask 收窄尚未实现。
- `docs/cross-platform/windows.md` 和 `docs/cross-platform/ipc-service.md` 已记录该限制；`tests/platform_layout.rs`
  增加守护，避免后续把 DACL hardening 误读成 client access mask hardening。
- 下一步如果要收窄 client mask，需要单独设计 raw `CreateFileW` + overlapped handle -> Tokio pipe client
  的路径，或等待/引入支持 explicit desired access 的 Tokio API；该实现必须重新跑 Windows IPC smoke。

Phase 10w Windows elevation split 修复已完成：

- 用户手动跑交叉矩阵发现：medium daemon 运行时 elevated `service status` 显示
  `daemon: not running`，且 elevated `--daemon` 会独立启动；反向 elevated daemon + medium client
  行为相同。这证明 Phase 10r 的 scope 选择把同一用户同一桌面拆成了两个 runtime。
- 根因判断：使用 `TokenStatistics.AuthenticationId` 作为 scope 输入时，elevated token 和 medium token
  可能拿到不同 LUID；这不符合 Windows user-session daemon 设计。
- 修复方向：Windows endpoint/mutex scope 改为当前 user SID + token logon SID
  (`SE_GROUP_LOGON_ID`) 的 SHA-256 prefix；logon SID 使同一交互式登录会话内 elevated/medium
  共享 endpoint，同时仍保留不同用户/不同登录会话隔离。
- Windows named mutex 创建改用与 Named Pipe 一致的 current-user security descriptor，避免 scope
  名字统一后仍受默认 DACL 差异影响。
- 验证结果：管理员侧 runtime smoke 通过；用户确认修复后两组交叉矩阵均符合预期：
  normal daemon + admin client 能看到 running daemon，第二个 admin `--daemon` 被拒绝；
  admin daemon + normal client 能看到 running daemon，第二个 normal `--daemon` 被拒绝。
- 已跑验证：`cargo fmt --check`、`cargo test --target x86_64-pc-windows-msvc windows_identity::tests`、
  `cargo test --target x86_64-pc-windows-msvc platform::lifecycle::imp::tests`、
  `cargo test --target x86_64-pc-windows-msvc ipc::transport::imp::tests`、
  `cargo build --target x86_64-pc-windows-msvc`、`cargo test --test platform_layout`。
- Capability 在 cross-user 第二账号/VM 隔离补齐前仍不得升级为 `available`。

Phase 10v Windows IPC boundary smoke 第一轮结果已记录：

- Elevated/elevated：当前 Codex session 为 `High Mandatory Level`；`shuo.exe --daemon`
  可保持运行，elevated `service status` exit 0 并返回
  `daemon: running ... state=Idle recording=-`；第二个 elevated `--daemon` exit 1，
  输出 `another shuo daemon is already starting or running`。
- Elevated busy smoke：elevated daemon 下并发 20 个 elevated `service status` job，20/20 exit 0，
  daemon 仍保持 running；日志目录
  `C:\Users\hza2002\AppData\Local\Temp\shuohua-phase10v-boundary\elev-busy-rerun`。
- Medium/medium：用户在普通 PowerShell 运行生成的
  `run-medium-boundary-smoke.ps1`，输出确认 `Medium Mandatory Level`；medium daemon +
  medium `service status` exit 0；第二个 medium `--daemon` exit 1；20 个并发 medium
  `service status` 全部 exit 0；日志目录
  `C:\Users\hza2002\AppData\Local\Temp\shuohua-phase10v-boundary\medium-manual`。
- Explorer open/reveal：`explorer.exe` 对 `%APPDATA%\Shuohua`、`%LOCALAPPDATA%\Shuohua`、
  `%APPDATA%\Shuohua\config.toml` reveal 的进程 exit code 仍为 1，但用户目视确认窗口已打开/
  reveal 生效；后续不要仅用 `explorer.exe` exit code 判断失败。
- Phase 10v 当时仍未验证 elevated daemon + medium client、medium daemon + elevated client 的交叉矩阵；
  Phase 10w 已补齐并修复 elevation split。cross-user 第二账号/VM 隔离仍未验证。
- Capability 结论不变：Windows `ipc.transport` / `daemon.single_instance` 仍保持
  `partial/runtime_not_verified`，至少等 cross-user 和交叉 elevation 矩阵补齐后再讨论升级。

Phase 10u Windows IPC boundary smoke checklist 已完成：

- `docs/cross-platform/windows-runtime-validation.md` 新增 Named Pipe Busy Smoke、Elevation Boundary
  Smoke、Cross-User Smoke 三段，作为 Phase 10r/10t 之后继续验证 Windows IPC scoping、安全边界和
  busy retry 的手动/半自动步骤。
- `tests/platform_layout.rs` 增加 checklist 顺序守护，避免 Windows runtime smoke 文档跳过
  Daemon/IPC、single instance、busy/elevation/cross-user 边界就进入后续桌面能力。
- 本 Windows elevated session 已跑同用户 busy smoke：启动 `shuo.exe --daemon` 后并发 20 个
  `shuo.exe service status` job，结果 `exit_0=20`，daemon 仍保持 running；日志目录为
  `C:\Users\hza2002\AppData\Local\Temp\shuohua-phase10u-busy`。
- 该阶段不声明 Windows IPC capability available：非 elevated/elevated 矩阵、cross-user 第二账号/VM
  验证、Explorer 窗口行为仍需要用户手动介入确认。
- 验证：`cargo fmt --check` 通过；
  `cargo test --test platform_layout windows_runtime_validation_checklist_stays_bottom_up` 通过。

Phase 10t Windows Named Pipe busy retry policy 已完成：

- Windows client connect 的 `ERROR_PIPE_BUSY` retry policy 抽为可测试边界：最多 20 次 open
  attempt，每次 busy 后等待 50ms。
- 该策略仍只覆盖短暂 server pipe instance 切换窗口；不启动 daemon，不实现 smart fallback，
  不代表 busy-pipe 压力测试或高并发 soak 已完成。
- 验证范围：Windows unit test 固定 retry 边界，仍需真实 runtime busy-pipe 压力和
  elevated/non-elevated/cross-user 验证。

Phase 10s Windows runtime checklist command sync 已完成：

- `docs/cross-platform/windows-runtime-validation.md` 中 daemon 启动命令从过时的
  `.\shuo.exe daemon` 修正为实际 CLI 入口 `.\shuo.exe --daemon`。
- checklist 中 Named Pipe 说明同步 Phase 10r 现状：endpoint scoping 和 current-user DACL 已做第一轮
  smoke，但 capability 仍保持 `partial/runtime_not_verified`，等待 cross-user、elevated/non-elevated、
  busy-pipe 和 long-running 验证。
- 验证：`cargo fmt --check` 通过；
  `cargo test --test platform_layout windows_runtime_validation_checklist_stays_bottom_up` 通过。

Phase 10r Windows Named Pipe endpoint scoping/security descriptor hardening 已完成：

- Windows Named Pipe endpoint 不再使用固定 `\\.\pipe\shuohua`，改为当前 user SID + logon SID
  的 SHA-256 prefix scope：`\\.\pipe\shuohua-<scope>`；raw SID 不进入对象名。
- Windows daemon named mutex 使用同一 scope：`Local\shuohua-daemon-<scope>`。
- Named Pipe server instance 创建时传入显式 SDDL security descriptor：
  current user SID、LocalSystem、Built-in Administrators；不授予 World/Everyone 或 Anonymous。
- 修复 runtime smoke 暴露的 Windows config diagnostics/inventory/root plan 路径偏差：运行时扫描
  `AppPaths::config_root()`，Windows 下为 `%APPDATA%\Shuohua`，不再把 `config_home` 再拼成小写
  `shuohua`。
- Windows runtime smoke 环境：Windows 11 Pro 10.0.26200 build 26200，PowerShell 7.5.5，
  `bill-win\hza2002`，最终 smoke shell 为 elevated。
- Windows runtime smoke 结果：
  - `shuo.exe --version` 输出 `shuo 0.2.0`。
  - `doctor` 扫描 `C:\Users\hza2002\AppData\Roaming\Shuohua`，仍因本机模板 secret 空值、
    无默认输入设备、权限探针等返回 1；这不代表 IPC smoke 失败。
  - `service status` 在 daemon 未运行时 exit 0，只打印 daemon not running + windows.user dry-run。
  - `shuo.exe --daemon` 可以保持运行；`service status` 通过 scoped Named Pipe 返回
    `daemon: running pid=33512 uptime=1s state=Idle recording=-`。
  - 第二个 `shuo.exe --daemon` exit 1，并输出
    `another shuo daemon is already starting or running`。
  - Explorer direct open/reveal 命令不挂起，但工具会话中 `explorer.exe` 仍快速返回 1；
    窗口行为未人工确认。
- Phase 10r 仍未完成：cross-user 验证、elevated/non-elevated 行为矩阵、busy-pipe 压力测试、
  client access mask 收窄、long-running soak。Windows `ipc.transport` / `daemon.single_instance`
  capability 仍必须保持 `partial/runtime_not_verified`。

Phase 10q Windows native build/test and first core runtime smoke 已完成：

- Windows 原生仓库路径：`C:\Users\hza2002\repo\shuohua`（请求里的 `C:\dev\shuohua` 不存在）。
- Windows 工具链：stable MSVC，`rustc -Vv` host 为 `x86_64-pc-windows-msvc`，`cl.exe` /
  `link.exe` 来自 VS 2022 Community MSVC 14.35。
- Windows build/test 修复点：
  - 非 macOS 不再编译/link `voice_activity_detector` / ONNX Runtime；Windows/Linux Silero VAD
    保持 explicit unavailable stub，避免当前机器 MSVC STL symbol link failure 阻塞 core runtime。
  - 多处测试和 diagnostics 改为 Windows-safe path/cfg 行为，避免 Unix-only 假设。
  - Windows IPC tests 使用合法 Named Pipe endpoint。
  - Windows/Linux overlay skeleton 的 daemon runtime 改为 no-op drain，capability 仍为
    `unsupported`，不实现 overlay。
  - 非 macOS hotkey backend 启动 idle placeholder，让 daemon core IPC 可以运行；capability
    仍为 `unsupported`，不实现 hotkey/suppression。
  - Windows `service status` 先查询 daemon `DaemonStatus`，再打印 user-session dry-run strategy；
    不安装 Task Scheduler/SCM，不启动 service。
  - 非 macOS binary/library crate-level 允许当前 skeleton surface 的 dead code/unused imports，
    用于让 Windows `clippy -D warnings` 通过；macOS 严格度不变。
- Windows runtime smoke 结果：
  - `shuo.exe --version` 输出 `shuo 0.2.0`。
  - `%APPDATA%\Shuohua` 用 `config-template` 创建；`%LOCALAPPDATA%\Shuohua` 在 smoke 中确认/创建。
  - `shuo.exe --daemon` 可以保持运行；`doctor` 能通过 Named Pipe 返回
    `daemon: OK pid=... state=Idle`。
  - 第二个 `shuo.exe --daemon` 明确失败：`another shuo daemon is already starting or running`。
  - `shuo.exe service status` 能显示运行中 daemon，并继续打印
    `windows.user: dry-run strategy=user_session_logon_task ... install_start=unsupported`。
  - `doctor` 仍因当前机器模板 secret 空值、无默认输入设备、权限探针等返回 1；这是本机环境/
    后续 backend 问题，不代表 IPC smoke 失败。
  - Explorer direct open/reveal 命令不挂起，但工具会话中 `explorer.exe` 快速返回 1；窗口行为未人工确认。
- 已知风险：
  - Windows overlay/hotkey/audio/clipboard/paste 仍未实现，不要把 daemon core smoke 解读为这些能力可用。
  - `windows-runtime-validation.md` 仍写了过时的 `shuo.exe daemon` 子命令；实际 CLI 入口是
    `shuo.exe --daemon`。

Phase 10m Windows Development Design Baseline 已完成：新增 `docs/cross-platform/windows.md`，
记录 Windows per-user desktop app 方向、AppData/LocalAppData 文件布局、Named Pipe 安全、
user-session daemon lifecycle、Task Scheduler startup 边界、audio/hotkey/clipboard/overlay 路线、
artifact 策略、runtime 验证顺序和需要用户介入的 stop points。
Phase 10m1 App Data Ownership Baseline 已完成：新增 `docs/cross-platform/app-data.md`，
明确 CLI、daemon、TUI、GUI 和 packaged desktop app 默认共享同一套 product data root；
package/app-private data 只保存 GUI/runtime 私有状态。macOS 配置可继续保持终端友好的
`~/.config/shuohua`；Windows product config/state 仍走 `%APPDATA%\Shuohua` /
`%LOCALAPPDATA%\Shuohua`。Phase 10n Windows Runtime Validation Checklist 已完成：
新增 `docs/cross-platform/windows-runtime-validation.md`，第一版只覆盖 artifact identity、
product data paths、daemon/client IPC、single instance、service dry-run 和 Explorer open/reveal；
不验证 audio/overlay/hotkey/clipboard/paste。下一步不要继续打磨 GUI placeholder；优先做
Windows path/config/state backend 和 Windows 本地开发链路。
Phase 10o Windows Path/Config/State Backend 已完成：`src/paths.rs` 新增 `AppPaths` product path
facade，config path helpers 和 `StateDirs` 改走该 facade；Windows target 使用 known-folder API
优先解析 Roaming/Local AppData，环境变量仅作为 fallback。该阶段仍只证明 compile/cfg 边界，
真实目录解析、package redirection、目录创建时机和 elevated/non-elevated 行为需要 Windows runtime
checklist。
Phase 10p Windows Local Development Setup 已完成：不使用 GitHub Actions 编译 Windows artifact，
因为 CI turnaround 太慢。Windows 机器作为本地开发/build/runtime 测试环境，通过 GitHub 同步代码。
`.github/workflows/ci.yml` 不再包含 `windows-artifact` job；`docs/cross-platform/windows-local-dev.md`
记录 Windows 本地 toolchain、Git sync、build/test 和结果回传流程。
Windows IPC capability 诊断已与 Phase 3c 同步：Windows target 使用 Tokio Named Pipe transport
编译通过，`ipc.transport` 静态 capability 报 `partial/named_pipe/runtime_not_verified`；runtime/ACL/
smart fallback 仍需 Windows 实机或 VM 验证。
Phase 10c Docker/cross Linux check baseline 已完成：macOS 主机使用 Docker/cross 负责 Linux
sysroot 和 C toolchain，`make check-linux-cross` 可通过；这只证明 Linux compile/cfg 边界，
不代表 Linux runtime 可用。
Phase 10i Audio Convert Facade 已完成：retained audio conversion 从 `voice::audio`
移动到 `platform::audio_convert` facade。macOS 保持 `/usr/bin/afconvert` 参数和 cleanup 语义；
Linux/Windows 暂时返回 explicit unsupported，直到选定 `ffmpeg`、`flac`/`lame`、纯 Rust encoder
或其他 backend 并在目标系统验证。该阶段不改变 retained audio 文件命名、history schema、
recorder WAV 写入或 `record_audio = "off"` 行为。
Phase 10j Windows Lifecycle Primitive Compile Backend 已完成：Windows `platform::lifecycle`
改为 Win32 named mutex / `OpenProcess` compile backend，capability 标记为
`partial/runtime_not_verified`；不实现 Windows service、smart fallback、daemon auto-start、
ACL/security descriptor hardening 或 runtime validation claims。
Phase 10k Windows Service Manager Dry-Run Status Skeleton 已完成：Windows
`platform::service` 增加 dry-run/status backend，`install` / `uninstall` / `start` / `stop` /
`restart` 仍 unsupported，不调用 Task Scheduler、SCM、PowerShell 或 registry APIs。
Phase 10l Non-macOS Desktop Capability Truthfulness 已完成：Linux/Windows
desktop capability 静态快照同步现有 facade 行为；不实现 hotkey、clipboard、text injection、
permission probe 或 active app runtime。

## 已完成事项

- Phase 0:
  - 新增 `docs/cross-platform/macos-baseline.md`，记录自动验证基线、macOS 手动验证 checklist、
    当前允许的 macOS-only 边界和后续阶段要处理的遗留边界。
  - 在 `docs/cross-platform/README.md` 增加 macOS baseline 阅读路由。
  - 扩展 `tests/platform_layout.rs`，保护 shared platform facade 和 macOS-only import 边界。
- Phase 1:
  - 新增 `src/platform/capability.rs`，提供共享 capability/status 类型和静态快照。
  - macOS 快照映射现有 backend；非 macOS 快照返回 `unsupported` +
    `backend_not_implemented`。
  - `shuo doctor` 只读打印 capability summary，不改变错误/警告计数或控制流。
- Phase 2:
  - 稳定 config/theme 跨平台规则，starter config 不默认输出 `[dev]`。
  - theme schema 增加受控的 `overlay.windows.material` / `overlay.linux.material` future 平台字段。
- Phase 3:
  - 新增 `src/ipc/transport.rs`，集中 macOS/Linux 当前 UDS endpoint、connect、bind、accept
    和 stale endpoint 清理。
  - `src/ipc/client.rs` / `src/ipc/server.rs` 不再直接 import `tokio::net::UnixStream` /
    `UnixListener`，JSON-line protocol 未改变。
- Phase 3c:
  - 更新 `docs/cross-platform/ipc-service.md`，记录 Windows Named Pipe transport compile backend
    的范围和未验证项。
  - Windows `ipc::transport` 从 placeholder `DuplexStream` 改为 Tokio
    `tokio::net::windows::named_pipe`。
  - server `accept()` 在当前 pipe instance 连接后创建下一条 pipe instance，再把已连接 stream
    交给既有 IPC server；client `connect()` 遇到 pipe busy 做短退避重试。
  - 该阶段不实现 Named Pipe ACL/security descriptor、不实现 Windows daemon single instance、
    不实现 smart fallback service 启动，也不声明 Windows runtime 可用。
- Windows IPC capability sync:
  - 更新 `docs/cross-platform/platform-capabilities.md`，记录 Windows `ipc.transport` 从默认
    `unsupported` 覆盖为 `partial`。
  - `current_platform_capabilities()` 在 Windows target 上保留其他 capability 默认 unsupported，
    只把 `ipc.transport` 标记为 backend `named_pipe`、reason `runtime_not_verified`、next step
    `Validate Named Pipe transport on Windows`。
  - `tests/platform_layout.rs` 增加静态守护，避免 Windows Named Pipe compile backend 已存在但
    capability 仍误报 unsupported。
- Phase 10c:
  - `Makefile` 新增 `make check-linux-cross`，执行
    带 `host.docker.internal:7890` 代理覆盖的
    `DOCKER_DEFAULT_PLATFORM=linux/amd64 cross check --target x86_64-unknown-linux-gnu`。
  - `Cargo.toml` 把 `voice_activity_detector` 改成非 Linux target dependency；Linux target 不再依赖
    `voice_activity_detector`，`src/voice/silero.rs` 在 Linux 下提供同名 unavailable stub，macOS/Windows
    真实 Silero 行为不变。这避免 Linux cross check 触发 `ort-sys/download-binaries` ->
    `ureq/native-tls` -> `openssl-sys` build-time 链路。
  - 新增 `Cross.toml`，只为 Linux GNU container 安装 `pkg-config libasound2-dev`，满足 `cpal` Linux
    ALSA backend 的 `alsa-sys` build script。
  - 更新 `docs/cross-platform/development-plan.md`，记录 macOS-hosted Linux check 应优先走
    Docker/cross，普通 `make check-linux` 仍需要本机 Linux C cross compiler/sysroot。
  - 当前本机探测：`cross 0.2.5` 已安装，Docker daemon 已运行，`docker info` 为
    `27.5.1 linux/aarch64`，Rust host 为 `aarch64-apple-darwin`。
  - 当前本机已安装 `stable-x86_64-unknown-linux-gnu --force-non-host`；`cross check` 已进入 Docker
    编译路径。
- Phase 10d:
  - 更新 `docs/cross-platform/platform-capabilities.md` 和
    `docs/cross-platform/development-plan.md`，记录 Linux compile-time capability sync 范围。
  - `current_platform_capabilities()` 在 Linux target 下不再全量返回 generic unsupported：
    `ipc.transport`、`daemon.single_instance`、`process.probe` 标记为 `available/compile_checked`；
    `audio.capture` 标记为 `partial/cpal_alsa/compile_checked`；`service.manager` 保持
    `unsupported/systemd_user_skeleton/backend_not_implemented`。
  - 该阶段不实现 systemd user service，不启动 Linux daemon，不验证 Linux audio device/permission，
    不实现 desktop hotkey/clipboard/text injection。
- Phase 10e:
  - 更新 `docs/cross-platform/ipc-service.md`、`docs/cross-platform/development-plan.md` 和
    `docs/cross-platform/overview.md`，记录 Linux systemd user dry-run/status skeleton 范围。
  - `src/platform/service.rs` 新增 Linux backend：`status()` 打印 daemon IPC 状态和
    `systemd.user: dry-run` unit/path/ExecStart 信息。
  - Linux backend 可以生成 systemd user unit body，baseline 为当前 executable + `--daemon`、
    `Restart=on-failure`、`RestartSec=2s` 和 `WantedBy=default.target`。
  - `install` / `uninstall` / `start` / `stop` / `restart` 继续返回明确 unsupported；该阶段不写
    unit 文件、不调用 `systemctl --user`、不新增 CLI flags。
- Phase 10f:
  - 更新 `docs/cross-platform/platform-capabilities.md`、`docs/cross-platform/development-plan.md`
    和 `docs/cross-platform/overview.md`，记录 Linux service manager capability sync。
  - `src/platform/capability.rs` 中 Linux `service.manager` 从
    `unsupported/systemd_user_skeleton/backend_not_implemented` 改为
    `partial/systemd_user_dry_run/dry_run_status_only`。
  - 该阶段只同步 doctor/TUI 使用的静态诊断 truthfulness；不实现 systemd install/start/stop/restart，
    不写 unit 文件、不调用 `systemctl --user`。
- Phase 10g:
  - 更新 `docs/cross-platform/platform-capabilities.md`、`docs/cross-platform/development-plan.md`
    和 `docs/cross-platform/overview.md`，记录 path open/reveal facade。
  - 新增 `src/platform/path.rs`，集中 `open_path()` / `reveal_path()`：
    macOS 继续使用 `open` / `open -R`；Linux 使用 `xdg-open`，reveal file fallback 到父目录；
    Windows/其他平台继续明确 unsupported。
  - `src/tui/audio.rs` 和 `src/tui/config_actions.rs` 不再直接调用 macOS `open` 命令；既有 audio
    path safety、config reveal 选择、`$VISUAL` / `$EDITOR` 优先级不变。
  - Linux `path.open_reveal` 静态 capability 同步为 `partial/xdg_open/reveal_opens_parent_dir`。
- Phase 10h:
  - 更新 `docs/cross-platform/platform-capabilities.md`、`docs/cross-platform/development-plan.md`
    和 `docs/cross-platform/overview.md`，记录 Windows path open/reveal compile backend。
  - `src/platform/path.rs` 新增 Windows backend：`open_path()` 使用 `explorer.exe <path>`，
    `reveal_path()` 使用 `explorer.exe /select,<path>`。
  - Windows `path.open_reveal` 静态 capability 同步为 `partial/explorer/runtime_not_verified`。
  - 该阶段只证明 Windows target 编译边界；真实 explorer 行为、路径 quoting、UNC、焦点和会话
    仍需 Windows VM/实机验证。
- Phase 10i:
  - 更新 `docs/cross-platform/platform-capabilities.md`、`docs/cross-platform/development-plan.md`
    和 `docs/cross-platform/overview.md`，记录 retained audio conversion facade。
  - 新增 `src/platform/audio_convert.rs`，集中 retained audio 转换：
    macOS 继续使用 `/usr/bin/afconvert`，Linux/Windows 返回明确 unsupported。
  - `src/voice/audio.rs` 不再直接持有 `afconvert` 命令、参数或 `std::process::Command`，finish
    路径改走 `platform::audio_convert::convert_retained_audio()`，原有 temp/final cleanup 语义保持。
  - 该阶段不改变 retained audio 文件命名、history schema、recorder WAV 写入或
    `record_audio = "off"` 行为。
- Phase 10j:
  - 更新 `docs/cross-platform/platform-capabilities.md`、`docs/cross-platform/development-plan.md`、
    `docs/cross-platform/ipc-service.md` 和 `docs/cross-platform/overview.md`，记录 Windows lifecycle
    primitive compile backend。
  - `src/platform/lifecycle.rs` 的 Windows backend 从 pure unsupported placeholder 改为 Win32
    named mutex daemon guard 和 `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` process probe。
  - Windows `daemon.single_instance` capability 标记为 `partial/named_mutex/runtime_not_verified`；
    `process.probe` 标记为 `partial/open_process_probe/runtime_not_verified`。
  - 新增 Windows-only `windows-sys` dependency，只启用 `Win32_Foundation` 和
    `Win32_System_Threading` feature。
  - 该阶段不实现 Windows service manager、smart fallback、daemon auto-start、Named Pipe ACL 或
    runtime validation。
- Phase 10k:
  - 更新 `docs/cross-platform/platform-capabilities.md`、`docs/cross-platform/development-plan.md`、
    `docs/cross-platform/ipc-service.md` 和 `docs/cross-platform/overview.md`，记录 Windows service
    manager dry-run/status skeleton。
  - `src/platform/service.rs` 新增 Windows backend：`status()` 打印 daemon not running 和
    `windows.user: dry-run strategy=user_session_logon_task command=... install_start=unsupported`。
  - Windows `service.manager` capability 标记为
    `partial/windows_user_dry_run/dry_run_status_only`。
  - `install` / `uninstall` / `start` / `stop` / `restart` 继续返回明确 unsupported；该阶段不调用
    Task Scheduler、SCM、PowerShell 或 registry APIs，不写文件，不实现 smart fallback。
- Phase 10l:
  - 更新 `docs/cross-platform/platform-capabilities.md`、`docs/cross-platform/development-plan.md`
    和 `docs/cross-platform/overview.md`，记录 Linux/Windows desktop capability truthfulness。
  - `src/platform/capability.rs` 新增 `non_macos_desktop_capabilities()`，Linux/Windows 均显式标记：
    `desktop.hotkey`、`desktop.hotkey_suppression`、`desktop.clipboard`、`desktop.text_injection`
    为 `unsupported/backend_not_implemented`；`desktop.active_app` 为
    `degraded/default_context/default_context_only`；`desktop.permissions` 为
    `unavailable/permission_probe_missing`。
  - 该阶段不实现 Linux/Windows hotkey、clipboard、text injection、active app 或 permission runtime。
- Phase 4a:
  - 更新 `docs/cross-platform/ipc-service.md`，把 Phase 4 拆成 lock/process probe facade 和
    后续 service manager facade。
  - 新增 `src/platform/lifecycle.rs`，集中 daemon lock file + `flock` 和 process probe
    `kill(pid, 0)` 语义。
  - 删除旧 `src/daemon/lock.rs`，`daemon::process` 改用 `platform::lifecycle::acquire_daemon_lock()`。
  - `cli::service::macos` 的 wait-for-exit 改用 `platform::lifecycle::process_exists()`，
    macOS stop/restart/status 用户可见语义不变。
  - `tests/platform_layout.rs` 增加 daemon lifecycle primitive import 边界测试。
- Phase 4b:
  - 更新 `docs/cross-platform/ipc-service.md`，记录 `platform::service` facade 边界。
  - 新增 `src/platform/service.rs`，集中 service manager backend 选择；macOS backend 继续使用
    launchd user agent。
  - `src/cli/service/mod.rs` 保留 clap command、命令分发和 `launchd_status()` 兼容入口，不再
    拥有 launchd 或 unsupported backend 文件。
  - 删除旧 `src/cli/service/macos.rs` / `src/cli/service/unsupported.rs`。
  - `tests/platform_layout.rs` 增加 service manager import 边界测试。
- Phase 5a:
  - 更新 `docs/cross-platform/platform-capabilities.md`，把 Phase 5 拆成 5a desktop facade 和
    5b hotkey provider facade。
  - 新增 `src/platform/desktop.rs`，聚合 active app、clipboard、text injection 和 permission
    primitives。
  - `voice::dispatch`、`voice::engine`、`platform::daemon`、`tui::history` 和 `cli::doctor`
    改用 `platform::desktop`。
  - 删除 `src/post/app_context.rs`；`post::AppContext` 保留为 post pipeline 数据模型，
    前台 App 查询归 desktop capability。
  - `tests/platform_layout.rs` 增加 desktop facade import 边界测试。
- Phase 5b:
  - 更新 `docs/cross-platform/platform-capabilities.md`，记录 hotkey provider facade 的边界。
  - 新增 `src/platform/hotkey.rs`，集中 hotkey provider backend 选择、OS thread spawn 和
    非 macOS unsupported fallback。
  - `src/platform/daemon.rs` 不再直接知道 `provider_darwin`、thread 名称或 unsupported 文案。
  - macOS 仍调用 `hotkey::provider_darwin::run()`；CGEventTap callback、pipe wire format、
    `Suppressor` 和 `TrackerSet` 未改变。
  - `tests/platform_layout.rs` 增加 hotkey provider facade import 边界测试。
- Phase 6a:
  - 更新 `docs/cross-platform/overlay.md` 和 `docs/modules/overlay.md`，记录 renderer facade
    边界。
  - 新增 `src/overlay/renderer.rs`，集中 overlay renderer backend 选择和非 macOS
    unsupported fallback。
  - `src/overlay/mod.rs` 的 `run()` 保持上层 API 不变，只转发到 `overlay::renderer`。
  - macOS backend 仍调用 `overlay::macos::run()`；AppKit view/chrome/icon_fx、动画、
    窗口层级、focused window 锚定和 material fallback 未改变。
  - `tests/platform_layout.rs` 增加 overlay renderer facade import 边界测试。
- Phase 6b:
  - 更新 `docs/cross-platform/overlay.md`、`docs/modules/overlay.md`、
    `docs/cross-platform/platform-capabilities.md` 和 `docs/cross-platform/overview.md`，
    记录 renderer capability skeleton 边界。
  - `src/overlay/renderer.rs` 新增只读 `renderer_capabilities()` 静态快照，复用
    `platform::capability` 的 `CapabilityStatus` / `CapabilityId` / status kind。
  - 新增 `MaterialPreference` 和 `MATERIAL_FALLBACK_ORDER`，固定
    `liquid_glass -> blurred_glass -> translucent -> solid` 的建模顺序。
  - macOS snapshot 描述当前 AppKit backend；非 macOS 仍是 structured unsupported。
  - macOS `overlay::run()` 仍调用 `overlay::macos::run()`；未修改 AppKit renderer、
    `OverlayCmd`、`OverlayModel`、layout 或 theme parser。
  - `tests/platform_layout.rs` 增加 renderer capability skeleton 边界测试。
- Phase 6c:
  - 更新 `docs/cross-platform/overlay.md`、`docs/cross-platform/platform-capabilities.md` 和
    `docs/cross-platform/overview.md`，记录 doctor 只读消费 renderer capability snapshot。
  - `src/overlay/mod.rs` 对 crate 内暴露 `renderer_capabilities()`。
  - `src/cli/doctor.rs` 的 capability summary 先读全局静态快照，再用 renderer snapshot
    覆盖同 `CapabilityId` 的 overlay 条目。
  - doctor 错误/警告计数、退出码、IPC/daemon/overlay 运行路径不变；TUI/GUI 未接入。
  - `tests/platform_layout.rs` 增加 renderer capability 仅由 doctor 消费的边界测试。
- Phase 7a:
  - 更新 `docs/cross-platform/overlay.md`，基于 Microsoft 文档记录 Windows overlay PoC
    baseline：Win32 popup/top-level window、extended styles、layered alpha、SetWindowPos
    topmost、WM_NCHITTEST click-through、Mica/DWM backdrop 降级判断和 capture exclusion。
  - 更新 `docs/cross-platform/development-plan.md`，把 Phase 7 拆出 7a 文档化 baseline。
  - 更新 `docs/cross-platform/overview.md`，记录 Phase 7a 当前状态。
  - 未新增 Windows renderer 文件，未引入依赖，未修改 macOS overlay 或 daemon 热路径。
- Phase 8a:
  - 更新 `docs/cross-platform/overlay.md`，基于 Wayland core/xdg-shell、wlr layer-shell、
    GTK Layer Shell、KDE LayerShellQt/KDE plasma shell protocol 和 GNOME Mutter issue
    记录 Linux Wayland overlay PoC baseline。
  - 记录 wlroots/KDE/GNOME/X11 的验证 checklist，并明确 GNOME Wayland 和普通 xdg-shell
    不应假设支持任意置顶 overlay。
  - 更新 `docs/cross-platform/development-plan.md`，把 Phase 8 拆出 8a 文档化 baseline。
  - 更新 `docs/cross-platform/overview.md`，记录 Phase 8a 当前状态。
  - 未新增 Linux renderer 文件，未引入 Wayland crate，未修改 macOS overlay 或 daemon 热路径。
- Phase 9a:
  - 更新 `docs/cross-platform/gui.md`，基于 Tauri v2 文档记录 GUI PoC baseline：
    独立按需 client、command/event 桥接、permissions/capabilities、sidecar 非默认路线、
    release build/bundle 指标和 TUI 回退。
  - 更新 `docs/cross-platform/development-plan.md`，把 Phase 9 拆出 9a 文档化 baseline。
  - 更新 `docs/cross-platform/overview.md`，记录 Phase 9a 当前状态。
  - 未新增 Tauri workspace，未引入 WebView runtime，未修改 daemon/CLI/TUI。
- Phase 9b:
  - 更新 `docs/cross-platform/gui.md`，记录共享 daemon client API 边界：只封装现有
    `ipc::protocol::Command` / `Event`，不新增 wire shape，不 bump `PROTO_VERSION`。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，
    记录 Phase 9b 的范围和状态。
  - 新增 `src/client_api.rs`，作为 TUI 和后续 GUI backend 复用的 daemon client 入口。
  - `src/tui/mod.rs` 改为通过 `client_api::DaemonClient` 获取 client 类型，startup command
    通过 `client_api::subscribe_command()` 构造；TUI 行为和 IPC protocol 不变。
  - `tests/platform_layout.rs` 增加 GUI client API 边界测试，禁止 daemon/TUI/shared client
    path 引入 Tauri、WRY、WebView 或 `tao` token，并确认 `Cargo.toml` 未新增相关依赖。
- Phase 9c:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI 首屏 helper 边界：request helper 只返回
    现有 `Command`，response classifier 只分类现有 `Event`，不做本地化、不读取
    config/history 文件、不生成 frontend view model。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，
    记录 Phase 9c 的范围和状态。
  - `src/client_api.rs` 增加 `first_screen_commands(history_limit)`，映射到
    `Subscribe`、`DaemonStatus`、`GetHistory` 和 `GetHistoryStats`。
  - `src/client_api.rs` 增加 `FirstScreenEvent` 和 `classify_first_screen_event()`，把
    `Snapshot`、`DaemonStatus`、`History`、`HistoryStats`、`HistoryChanged` 和 `Error`
    分类为 GUI backend 可消费的首屏输入。
  - `src/main.rs` 将 `client_api` 公开为 crate 边界，供后续 GUI backend 复用；未新增
    Tauri workspace 或 GUI runtime 依赖。
  - `tests/platform_layout.rs` 增加首屏 helper 架构测试，确认 helper 仍位于 `client_api`，
    不拥有 protocol version，也不引入 Tauri/WRY/WebView/`tao` token。
- Phase 9d:
  - 更新 `docs/cross-platform/gui.md`，明确当前 crate 只有 binary target，`client_api`
    仍是 binary crate 内边界，不是外部 Tauri crate 可依赖的 library API。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，
    记录真正创建 Tauri workspace 前必须先做 library split 评审。
  - 记录 library split 的最小候选 surface：`client_api`、`ipc::client`、`ipc::protocol`、
    `ipc::transport` 和必要数据模型；禁止把 daemon runtime、hotkey、overlay、voice、
    AppKit 或 TUI 拉进 GUI backend 依赖树。
  - `tests/platform_layout.rs` 增加当前边界保护测试：没有 `src/lib.rs`、没有 Tauri workspace
    文件、`Cargo.toml` 仍只有既有 `shuo` binary target 且不含 GUI runtime 依赖。
- Phase 9e:
  - 更新 `docs/cross-platform/gui.md`，记录 library split audit baseline。
  - 记录最小候选 library surface：`client_api`、`ipc::client`、`ipc::protocol`、
    `ipc::transport` 和必要数据模型，足够后续 GUI backend 连接 daemon、发送首屏命令、
    接收并分类首屏事件。
  - 记录阻塞点：`ipc::protocol` 依赖 `history` / `state` 模型，不能只移动 protocol 文件；
    `ipc::transport` 当前是 Unix-only transport，Windows Named Pipe backend 仍属后续 IPC
    transport backend 阶段。
  - 继续禁止在 library split 前创建 Tauri workspace，避免复制 IPC 类型或绕过 `client_api`。
  - `tests/platform_layout.rs` 增加 audit 文档守卫，确认 GUI 文档记录最小 surface、阻塞点和
    禁止方向。
- Phase 9f:
  - 更新 `docs/cross-platform/gui.md`，记录最小 library split 的范围、禁止方向和验收标准。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9f 状态。
  - 新增 `src/lib.rs`，只公开 `client_api`、`history`、`ipc`、`paths`、`state`、
    `text_stats`。
  - `src/ipc/mod.rs` 的 library surface 只公开 `client`、`protocol`、`transport`；`ipc::server`
    留在 binary 的内联 `ipc` 模块中。
  - `src/main.rs` 继续挂载 `ipc::server`，daemon runtime 可用路径不变。
  - `tests/platform_layout.rs` 增加最小 library surface 守卫，并把旧 9d 测试调整为继续禁止
    Tauri workspace / GUI runtime 依赖。
  - 未新增 IPC command/event，未 bump `PROTO_VERSION`，未新增 Tauri/WRY/WebView 依赖。
- Phase 9g:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI client 连接状态骨架范围：只描述 client
    side 状态、recoverable problem kind 和 retry delay，不实现后台 reconnect loop。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9g 状态。
  - `src/client_api.rs` 新增 `DaemonConnectionState`、`DaemonConnectionProblemKind`、
    `DaemonConnectionProblem`、`DEFAULT_RECONNECT_DELAYS_MS`、`next_reconnect_delay_ms()`、
    `reconnecting_state()` 和 daemon connection problem helper。
  - retry delay 是纯函数、短序列且有上限；`reconnecting_state()` 的 attempt 计数在极大输入下
    饱和到 `u32::MAX`。
  - `tests/platform_layout.rs` 增加 reconnect skeleton 架构守卫，确认 daemon/TUI 还未消费该
    GUI 状态骨架，且未引入 runtime loop 或 GUI runtime。
  - 未新增 IPC command/event，未 bump `PROTO_VERSION`，未创建 Tauri workspace，未改变 TUI
    连接行为。
- Phase 9h:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI backend event bridge 骨架范围：只把既有
    daemon `Event`、connection state 和 recoverable connection problem 封装成 GUI backend
    可转发事件。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9h 状态。
  - `src/client_api.rs` 新增 `GuiBackendEvent<'a>`，以及
    `gui_backend_event_from_daemon_event()`、`gui_backend_event_from_connection_state()`、
    `gui_backend_event_from_connection_problem()`。
  - daemon event bridge 复用 `classify_first_screen_event()`；bridge 只持有引用，不 clone 大型
    history payload，不生成 frontend view model，不调用 Tauri event API。
  - `tests/platform_layout.rs` 增加 bridge 架构守卫，确认未引入 Tauri/WRY/WebView、runtime loop
    或 protocol ownership。
  - 未新增 IPC command/event，未 bump `PROTO_VERSION`，未创建 Tauri workspace，未改变 TUI
    连接行为。
- Phase 9i:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI 首屏 metrics/timing 纯模型边界：时间戳由后续
    GUI backend 传入，shared client API 只做纯计算、饱和差值和首屏 readiness 判定。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9i 状态。
  - `src/client_api.rs` 新增 `FirstScreenReadiness`、`FirstScreenTimingMarks`、
    `FirstScreenTiming` 和纯 `from_marks()` helper。
  - 首屏 ready 的最小判定要求 daemon status、history page 和 history stats 都到达；snapshot、
    history changed 和 recoverable error 不会单独让首屏 ready。
  - helper 不调用系统时间、timer、IPC、Tauri event API 或 metrics sink；未新增 IPC
    command/event，未 bump `PROTO_VERSION`，未创建 Tauri workspace，未改变 TUI 连接行为。
- Phase 9j:
  - 基于 Tauri v2 文档更新 `docs/cross-platform/gui.md`，记录 capabilities/permissions
    preflight：capabilities 将 permissions 授权给指定 windows/webviews，permissions 显式开启
    frontend 可访问 command/plugin，并可包含 scopes。
  - 明确 GUI PoC 只给主 window/webview 绑定最小 capability，只暴露 shuohua GUI backend 自有
    command；frontend 不直接访问 IPC transport、history/config 文件或 daemon implementation。
  - 明确 PoC 不默认启用 shell、filesystem、http、process、global shortcut、updater、sidecar
    管理等宽权限；`core:default` 不作为默认授权策略，创建 workspace 时需先列出实际所需
    permission。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9j 状态。
  - `tests/platform_layout.rs` 增加文档/架构守卫，确认权限 preflight 已记录，且仍无
    `src-tauri/**` workspace 文件或 Tauri/WRY/WebView runtime 依赖。
- Phase 9k:
  - 基于 Tauri v2 build/bundle 文档更新 `docs/cross-platform/gui.md`，记录创建最小 Tauri
    workspace 前的验收清单。
  - 明确下一阶段只允许新增最小 GUI app 骨架、主 window/webview、最小 capabilities 文件和
    调用 shared `client_api` 的 backend shell；禁止同时实现完整页面、onboarding、配置编辑器、
    service management、sidecar、复制 IPC 类型或 daemon runtime 依赖。
  - 记录 release 指标清单：bundle path/type、unsigned/signed 状态、cold start、首屏 ready、
    open GUI idle RSS/CPU、关闭 GUI 后 daemon 存活、daemon 未打开 GUI 时无 WebView/Tauri
    进程。
  - `tests/platform_layout.rs` 增加文档/架构守卫，确认 workspace 前验收清单已记录，且仍无
    `src-tauri/**` workspace 文件或 Tauri/WRY/WebView runtime 依赖。
- Phase 9l:
  - 更新 `docs/cross-platform/gui.md`，记录后续 GUI backend 的 connection supervisor task
    ownership：首次连接 daemon、发送 `first_screen_commands()`、订阅 daemon event、应用
    `reconnecting_state()` 退避并通过 `GuiBackendEvent` 转发状态。
  - 明确 supervisor 属于 GUI 进程，不进入 daemon、TUI 或 shared `client_api`；取消 owner 是
    GUI window/app lifecycle，旧 task 的 late event 必须由 session id/generation 丢弃。
  - 明确 reconnect 只处理 recoverable client-side 问题：connect failed、event stream closed、
    read failed；不自动启动 daemon、不安装或重启 service、不修改配置。
  - 明确 timer、spawn、channel、Tauri event emission、metrics sink 只属于后续 GUI backend；
    shared `client_api` 继续只提供纯状态、退避、event bridge 和 timing helper。
  - `tests/platform_layout.rs` 增加文档/架构守卫，确认 reconnect ownership 已记录，且
    `src/client_api.rs` 仍无 runtime/GUI token。
- Phase 9m:
  - 更新 `docs/cross-platform/gui.md`，记录最小 Tauri workspace skeleton 的允许文件、权限边界
    和禁止项。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9m 状态。
  - 新增 `src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`src-tauri/build.rs`、
    `src-tauri/src/main.rs`、`src-tauri/src/lib.rs` 和 `src-tauri/capabilities/default.json`。
  - `src-tauri/Cargo.toml` 是独立 `shuohua-gui` crate，使用 Tauri v2，并通过
    `shuohua = { path = ".." }` 依赖根 crate；root `Cargo.toml` 未加入 workspace 或 Tauri
    dependency。
  - capabilities 只绑定主 window，权限保持在 `core:event:default`；未启用 shell、filesystem、
    http、process、global shortcut、updater 或 sidecar。
  - `tests/platform_layout.rs` 增加 Phase 9m skeleton 隔离测试，并把旧 Phase 9d 守卫调整为
    继续保护 root runtime 不引入 GUI runtime，而不是禁止 `src-tauri/**` 存在。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 frontend view model 或 reconnect supervisor。
- Phase 9n:
  - 更新 `docs/cross-platform/gui.md`，记录最小 GUI backend shell 和静态 frontend placeholder
    的边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9n 状态。
  - `src-tauri/src/lib.rs` 增加本地 `gui_shell_metadata` Tauri command，并通过
    `tauri::generate_handler!` 注册到 builder；command 只返回静态 metadata。
  - 新增 `gui-dist/index.html`，作为 `frontendDist` 的最小静态 placeholder；不引入 npm/vite、
    frontend dependency、dev server config 或完整页面。
  - `src-tauri/tauri.conf.json` 显式使用既有 `../assets/icon/shuohua-icon-1024.png`，并设置
    `bundle.active=false`，让 `cargo check --manifest-path src-tauri/Cargo.toml` 能通过 Tauri
    `generate_context!()` 的编译期 icon 检查，但仍不做 bundle。
  - 新增 `src-tauri/Cargo.lock`，锁定独立 GUI app crate 的 Tauri 依赖；`.gitignore` 忽略
    Tauri build script 生成的 `src-tauri/gen/` schema 目录。
  - `tests/platform_layout.rs` 增加 Phase 9n 架构守卫，确认 GUI shell 不连接 daemon、不拥有
    runtime loop，且 root/daemon/TUI/client_api 不引入 GUI runtime token。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 Status/History/Diagnostics view model 或 reconnect supervisor。
- Phase 9o:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI first-screen request plan command 的边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9o 状态。
  - `src-tauri/src/lib.rs` 增加 `gui_first_screen_request_plan` Tauri command，复用
    `shuohua::client_api::first_screen_commands()` 生成首屏请求计划 summary。
  - request plan 只返回 command kind、history limit、requires daemon connection 和 transport
    opened=false；不创建 `DaemonClient`，不调用 `connect_default()`，不发送 IPC，不订阅 event
    stream。
  - `gui-dist/index.html` 展示 request plan command count/kinds 和静态连接字段；仍不实现真实
    Status/History/Diagnostics view model。
  - `tests/platform_layout.rs` 增加 Phase 9o 架构守卫，并调整 9n 守卫以允许 9o 在 `src-tauri`
    内对既有 `Command` 做 summary 映射。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 reconnect supervisor。
- Phase 9p:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI daemon status snapshot shape command 的边界：
    这是 shape preflight，不是真实 status client。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9p 范围和状态。
  - `src-tauri/src/lib.rs` 增加 `gui_daemon_status_snapshot` Tauri command，返回静态
    `connected=false`、`transport_opened=false`、`snapshot_available=false`、
    `state_label=disconnected`，并标记后续真实请求使用既有 `Command::DaemonStatus`。
  - `gui-dist/index.html` 展示 status snapshot shape；仍不实现真实 Status/History/Diagnostics
    view model。
  - `tests/platform_layout.rs` 增加 Phase 9p 架构守卫，确认 command 不创建 `DaemonClient`、
    不调用 `connect_default()`、不发送 IPC、不订阅 event stream、不启动 spawn/timer。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 reconnect supervisor 或 service management。
- Phase 9q:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI daemon status event mapper 的边界：只把调用方
    已拿到的既有 `Event::DaemonStatus` 映射成 Phase 9p 的 status snapshot response shape。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9q 范围和状态。
  - `src-tauri/src/lib.rs` 新增纯 `gui_daemon_status_snapshot_from_event()` mapper 和
    `wire_state_label()` helper；mapper 只处理 `Event::DaemonStatus`，其他 event 返回 `None`。
  - `GuiDaemonStatusSnapshot` 增加 `pid`、`uptime_ms`、`recording_id` 可选字段；9p 的
    `gui_daemon_status_snapshot` 继续通过 empty helper 返回未连接静态 shape。
  - 新增 Tauri crate 单元测试覆盖 `Event::DaemonStatus` 到 snapshot shape 的映射，以及
    `HistoryChanged` 不被误处理。
  - `tests/platform_layout.rs` 增加 Phase 9q 架构守卫，确认 mapper 不创建 `DaemonClient`、
    不调用 `connect_default()`、不发送 IPC、不订阅 event stream、不启动 spawn/timer。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现真实 status request、reconnect supervisor 或 service management。
- Phase 9r:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI daemon status one-shot request command 边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9r 范围和状态。
  - `src-tauri/src/lib.rs` 增加 `gui_daemon_status_request_once` Tauri command：显式调用时通过
    `DaemonClient::connect_default()` 连接现有 daemon IPC，发送既有 `Command::DaemonStatus`，
    用 `recv_until` 等待 `Event::DaemonStatus` 并复用 9q mapper 返回 snapshot shape。
  - 新增 `GuiDaemonStatusRequestError` recoverable error shape，覆盖 connect/write/read failure、
    daemon `Event::Error` 和 daemon closed。
  - placeholder `gui-dist/index.html` 不自动调用 one-shot command，避免打开静态页面时默认连接
    daemon。
  - `tests/platform_layout.rs` 增加 Phase 9r 架构守卫，确认 one-shot command 显式存在但不发送
    `Subscribe`、不订阅 event stream、不启动 spawn/timer/reconnect loop。
  - Tauri crate 单元测试覆盖 status event mapping 和 recoverable error shape。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 frontend Status view model、reconnect supervisor 或 service
    management。
- Phase 9s:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI history summary one-shot request command 边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9s 范围和状态。
  - `src-tauri/src/lib.rs` 增加 `gui_history_summary_request_once` Tauri command：显式调用时通过
    `DaemonClient::connect_default()` 连接现有 daemon IPC，发送既有
    `Command::GetHistory { limit, before: None, before_id: None, query: None }` 和
    `Command::GetHistoryStats`，用 `recv_until` 等待 `Event::History` / `Event::HistoryStats`
    并返回最小 history summary shape。
  - 新增 `GuiHistorySummaryRequestError` recoverable error shape，覆盖 connect/write/read failure、
    daemon `Event::Error` 和 daemon closed。
  - summary 只包含 page count、matched、aggregate stats、latest record id/status/text preview
    和 request metadata；不实现搜索、分页 cursor、详情、audio 管理、图表或本地化。
  - placeholder `gui-dist/index.html` 不自动调用 one-shot command，避免打开静态页面时默认连接
    daemon 或读取 history。
  - `tests/platform_layout.rs` 增加 Phase 9s 架构守卫，确认 one-shot command 显式存在但不发送
    `Subscribe`、不订阅 event stream、不启动 spawn/timer/reconnect loop。
  - Tauri crate 单元测试覆盖 history summary event mapping 和 recoverable error shape。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 frontend History view model、reconnect supervisor 或 service
    management。
- Phase 9t:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI first-screen summary one-shot request command
    边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9t 范围和状态。
  - `src-tauri/src/lib.rs` 增加 `gui_first_screen_summary_request_once` Tauri command：显式调用时
    通过一次 `DaemonClient::connect_default()` 连接现有 daemon IPC，发送既有
    `Command::DaemonStatus`、`Command::GetHistory { limit, before: None, before_id: None,
    query: None }` 和 `Command::GetHistoryStats`，用 `recv_until` 等待 `Event::DaemonStatus` /
    `Event::History` / `Event::HistoryStats` 并返回组合 first-screen summary shape。
  - summary 复用 9r status snapshot shape 和 9s history summary shape，并带 history limit、
    availability 和 request metadata；不实现 loading/retry UI、metrics 展示、event stream、
    搜索、详情、audio 管理或本地化。
  - 新增 `GuiFirstScreenSummaryRequestError` recoverable error shape，覆盖 connect/write/read
    failure、daemon `Event::Error` 和 daemon closed。
  - placeholder `gui-dist/index.html` 不自动调用 one-shot command，避免打开静态页面时默认连接
    daemon 或读取 history。
  - `tests/platform_layout.rs` 增加 Phase 9t 架构守卫，确认 one-shot command 显式存在但不发送
    `Subscribe`、不订阅 event stream、不启动 spawn/timer/reconnect loop。
  - Tauri crate 单元测试覆盖 first-screen summary event mapping 和 recoverable error shape。
  - 未运行 `tauri dev` / `tauri build` / `tauri bundle`，未启动 daemon/GUI，未新增 IPC
    command/event，未实现 frontend Status/History view model、reconnect supervisor 或 service
    management。
- Phase 9u:
  - 更新 `docs/cross-platform/gui.md`，记录 GUI first-screen summary request timing 的边界。
  - 更新 `docs/cross-platform/development-plan.md` 和 `docs/cross-platform/overview.md`，记录
    Phase 9u 范围和状态。
  - `src-tauri/src/lib.rs` 的 `GuiFirstScreenSummary` 增加 `timing` 字段，类型为
    `GuiFirstScreenSummaryTiming`，包含 `connectDurationMs`、`firstEventMs`、`readyMs` 和
    `requestDurationMs`。
  - `gui_first_screen_summary_request_once` 在本次显式 command invocation 内使用
    `std::time::Instant` 记录 request start、connect completed、first matched daemon event 和
    summary ready 的 elapsed milliseconds。
  - timing 只附着在 9t 的 first-screen summary response 上；不进入 daemon protocol、
    shared `client_api`、history、trace 或 metrics sink。
  - 未使用 `tokio::time`，未启动 timer task，未订阅 event stream，未实现 reconnect loop、
    loading/retry UI 或 frontend view model。
- Phase 10a:
  - `Makefile` 新增 `make check-windows` 和 `make check-linux`，作为跨平台 cfg/type 边界检查入口。
  - shared network clients 改为 target-specific TLS：Linux 使用 Rustls，非 Linux 保持 native TLS。
  - `shuo doctor` 的 platform capability summary 增加 unsupported/unavailable detail 行，包含
    backend、reason 和可选 next step，方便 skeleton 阶段诊断。
  - `tests/platform_layout.rs` 增加 network TLS 配置守护测试，避免 Linux check 路径重新引入
    OpenSSL-backed native TLS。
- Phase 10b:
  - 更新 `docs/cross-platform/development-plan.md`，记录 TUI capability diagnostics 的只读边界。
  - TUI Status 页新增 `Platform` 区块，合并 `current_platform_capabilities()` 和
    `overlay::renderer_capabilities()` 后显示 available/unsupported/unavailable/partial/degraded/unknown
    计数。
  - TUI capability detail 只列 non-available entries，展示 capability id、status、backend、reason
    和可选 next step。
  - `tests/platform_layout.rs` 更新 renderer capability consumer 边界：允许 doctor 和 TUI Status
    消费，继续禁止 GUI/WebView/IPC/daemon client/task 进入 TUI summary。

## 验证结果

- 已跑：`cargo test --test platform_layout daemon_lifecycle_primitives_live_behind_platform_facade`，通过。
- 已跑：`cargo test --test platform_layout service_manager_lives_behind_platform_facade`，通过。
- 已跑：`cargo test platform::service::`，通过 12 个测试。
- 已跑：`cargo test cli::service::`，通过 1 个测试。
- 已跑：`cargo test platform::lifecycle`，通过 2 个测试。
- Phase 4a 曾跑：`cargo test cli::service::macos::tests`，通过 12 个测试；Phase 4b 后这些
  测试已随实现迁移到 `platform::service::`。
- 已跑：`cargo test --test platform_layout desktop_capabilities_live_behind_platform_desktop_facade`，
  先红灯失败于缺少 `src/platform/desktop.rs`，实现后通过。
- 已跑：`cargo test --test platform_layout hotkey_provider_lives_behind_platform_hotkey_facade`，
  先红灯失败于缺少 `src/platform/hotkey.rs`，实现后通过。
- 已跑：`cargo test --test platform_layout overlay_renderer_lives_behind_renderer_facade`，
  先红灯失败于缺少 `src/overlay/renderer.rs`，实现后通过。
- 已跑：`cargo test --test platform_layout overlay_renderer_capabilities_live_with_renderer_facade`，
  先红灯失败于缺少 `renderer_capabilities`，实现后通过。
- 已跑：`cargo test overlay::renderer`，通过 3 个 renderer 单元测试。
- 已跑：`cargo test cli::doctor::tests`，通过 7 个测试。
- 已跑：`cargo test hotkey`，通过 81 个测试。
- 已跑：`cargo test overlay`，通过 45 个 unit tests，另外 integration tests 过滤项正常。
- 已跑：`cargo test --test doc_consistency`，通过 2 个测试。
- 已跑：`cargo test --test platform_layout`，通过 13 个测试。
- 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，通过。
  `cargo test` 覆盖：633 个 unit tests、5 个 `apple_helper_build` tests、
  1 个 `cli_runtime_boundary` test、2 个 `doc_consistency` tests、13 个 `platform_layout` tests、
  6 个 `theme_registry_build` tests。
- Phase 9b 已跑：`cargo test --test platform_layout gui_client_api_boundary_stays_out_of_daemon_hot_path`，
  先红灯失败于缺少 `src/client_api.rs`，实现后通过。
- Phase 9b 已跑：`cargo test client_api::tests`，通过 1 个 client API 单元测试。
- Phase 9b 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 9c 已跑：`cargo test client_api::tests`，先红灯失败于缺少
  `first_screen_commands`、`classify_first_screen_event` 和 `FirstScreenEvent`，实现后通过
  3 个 client API 单元测试。
- Phase 9c 已跑：`cargo test --test platform_layout gui_first_screen_helpers_live_in_client_api_without_gui_runtime`，
  通过。
- Phase 9c 已跑：`cargo test --test platform_layout`，通过 15 个测试。
- Phase 9c 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 9d 已跑：`cargo test --test platform_layout gui_library_boundary_is_not_split_before_design_review`，
  通过。
- Phase 9d 已跑：`cargo test --test platform_layout`，通过 16 个测试。
- Phase 9e 已跑：`cargo test --test platform_layout gui_library_split_audit_records_minimal_surface_and_blockers`，
  先红灯失败于缺少 Phase 9e 文档，补文档后通过。
- Phase 9e 已跑：`cargo test --test platform_layout`，通过 17 个测试。
- Phase 9f 已跑：`cargo test --test platform_layout gui_minimal_library_split_exposes_only_client_protocol_surface`，
  先红灯失败于缺少 `src/lib.rs`，实现后通过。
- Phase 9f 已跑：`cargo test client_api::tests`，通过。该命令同时覆盖 `src/lib.rs` 和
  `src/main.rs` 中的 client API 单元测试。
- Phase 9f 已跑：`cargo test --test platform_layout`，通过 18 个测试。
- Phase 9f 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：89 个 library unit tests、636 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、18 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9g 已跑：`cargo test client_api::tests::daemon_connection_state_models_bounded_reconnect_without_protocol_changes`，
  先红灯失败于缺少 reconnect state 类型和 helper，实现后通过。
- Phase 9g 已跑：`cargo test --test platform_layout gui_reconnect_state_skeleton_lives_in_client_api_without_runtime_loop`，
  通过。
- Phase 9g 已跑：`cargo test client_api::tests`，通过 4 个 client API 单元测试。
- Phase 9g 已跑：`cargo test --test platform_layout`，通过 19 个测试。
- Phase 9g 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：90 个 library unit tests、637 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、19 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9h 已跑：`cargo test client_api::tests::gui_backend_event_bridge_wraps_existing_client_api_shapes`，
  先红灯失败于缺少 `GuiBackendEvent` 和 bridge helper，实现后通过。
- Phase 9h 已跑：`cargo test --test platform_layout gui_backend_event_bridge_lives_in_client_api_without_gui_runtime`，
  通过。
- Phase 9h 已跑：`cargo test client_api::tests`，通过 5 个 client API 单元测试。
- Phase 9h 已跑：`cargo test --test platform_layout`，通过 20 个测试。
- Phase 9h 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：91 个 library unit tests、638 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、20 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9i 已跑：`cargo test client_api::tests::first_screen_timing_models_readiness_without_runtime_or_protocol_changes`，
  先红灯失败于缺少 `FirstScreenReadiness`、`FirstScreenTimingMarks` 和 `FirstScreenTiming`，
  实现后通过。
- Phase 9i 已跑：`cargo test --test platform_layout gui_first_screen_metrics_timing_stays_pure_client_api`，
  先红灯失败于缺少 Phase 9i client API token，实现后通过。
- Phase 9i 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、21 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9j 已跑：`cargo test --test platform_layout gui_tauri_permissions_preflight_is_documented_without_workspace`，
  通过。
- Phase 9j 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、22 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9k 已跑：`cargo test --test platform_layout gui_tauri_workspace_pre_creation_acceptance_is_documented_without_workspace`，
  先红灯失败于缺少连续的进程边界 token，补文档后通过。
- Phase 9k 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、23 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9l 已跑：`cargo test --test platform_layout gui_reconnect_supervisor_ownership_is_documented_without_runtime_loop`，
  先红灯失败于缺少稳定 `connection supervisor` 和 `read failed` 文档 token，补文档后通过。
- Phase 9l 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、24 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9m 已跑：`cargo test --test platform_layout gui_minimal_tauri_workspace_skeleton_is_isolated_from_root_runtime`，
  先红灯失败于缺少 `src-tauri/Cargo.toml`，实现后通过。
- Phase 9m 已跑：`rg -n "tauri|wry|webview|WebView|tao" Cargo.toml src/daemon src/tui src/client_api.rs`，
  无命中。
- Phase 9m 已跑：`cargo test --test platform_layout`，通过 25 个测试。
- Phase 9n 已跑：`cargo test --test platform_layout gui_backend_shell_placeholder_stays_local_to_tauri_app`，
  先红灯失败于缺少 `#[tauri::command]`，实现后通过。
- Phase 9n 已跑：`cargo test --test platform_layout gui_minimal_tauri_workspace_skeleton_is_isolated_from_root_runtime`，
  通过。
- Phase 9n 已跑：`cargo test --test platform_layout`，通过 26 个测试。
- Phase 9n 已跑：`cargo check --manifest-path src-tauri/Cargo.toml`。第一次失败于
  `generate_context!()` 找不到默认 `src-tauri/icons/icon.png`；改为显式使用已有
  `assets/icon/shuohua-icon-1024.png` 后通过。
- Phase 9n 已跑：`rg -n "tauri|wry|webview|WebView|tao" Cargo.toml src/daemon src/tui src/client_api.rs`，
  无命中。
- Phase 9n 已跑：`rg -n "connect_default|DaemonClient|ipc::client|Command::|Event::|tokio::spawn|tokio::time|std::thread::spawn" src-tauri`，
  无命中。
- Phase 9n 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、26 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9o 已跑：`cargo test --test platform_layout gui_first_screen_request_plan_reuses_client_api_without_sending_ipc`，
  先红灯失败于缺少 `gui_first_screen_request_plan`，实现后通过。
- Phase 9o 已跑：`cargo test --test platform_layout`，通过 27 个测试。
- Phase 9o 已跑：`cargo check --manifest-path src-tauri/Cargo.toml`，通过。
- Phase 9o 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 9o 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  639 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、27 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 9o 已跑：`rg -n "tauri|wry|webview|WebView|tao" Cargo.toml src/daemon src/tui src/client_api.rs`，
  无命中。
- Phase 9o 已跑：`rg -n "connect_default|DaemonClient|send_command|subscribe_events|tokio::spawn|tokio::time|std::thread::spawn" src-tauri`，
  无命中。
- Phase 9p 已跑：`cargo test --test platform_layout gui_daemon_status_snapshot_shape_does_not_send_ipc`，
  先红灯失败于缺少 `gui_daemon_status_snapshot`，实现后通过。
- Phase 9p 已跑：`cargo check --manifest-path src-tauri/Cargo.toml`，通过。
- Phase 9p 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、28 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 9q 已跑：`cargo test --manifest-path src-tauri/Cargo.toml daemon_status_event_maps_to_snapshot_shape_without_ipc`，
  先红灯失败于缺少 `gui_daemon_status_snapshot_from_event`，实现后通过。
- Phase 9q 已跑：`cargo test --test platform_layout gui_daemon_status_event_mapper_is_pure_and_local_to_tauri_app`，
  通过。
- Phase 9q 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml daemon_status_event_maps_to_snapshot_shape_without_ipc`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、29 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 `daemon_status_event_maps_to_snapshot_shape_without_ipc`。
- Phase 9r 已跑：`cargo test --test platform_layout gui_daemon_status_one_shot_request_is_explicit_and_bounded`，
  先红灯失败于缺少 `gui_daemon_status_request_once`，实现后通过。
- Phase 9r 已跑：`cargo test --manifest-path src-tauri/Cargo.toml daemon_status`，通过 2 个
  Tauri crate 单元测试。
- Phase 9r 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml daemon_status`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、30 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 2 个 daemon status tests。
- Phase 9s 已跑：`cargo test --test platform_layout gui_history_summary_one_shot_request_is_explicit_and_bounded`，
  先红灯失败于缺少 `gui_history_summary_request_once`，实现后通过。
- Phase 9s 已跑：`cargo test --manifest-path src-tauri/Cargo.toml history_summary`，通过 2 个
  Tauri crate 单元测试。
- Phase 9s 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml history_summary`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、31 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 2 个 history summary tests。
- Phase 9t 已跑：`cargo test --test platform_layout gui_first_screen_summary_one_shot_request_is_explicit_and_bounded`，
  先红灯失败于缺少 `gui_first_screen_summary_request_once`，实现后通过。
- Phase 9t 已跑：`cargo test --manifest-path src-tauri/Cargo.toml first_screen_summary`，通过 2 个
  Tauri crate 单元测试。
- Phase 9t 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_summary`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、32 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 2 个 first-screen summary tests。
- Phase 9u 已跑：`cargo test --test platform_layout gui_first_screen_summary_timing_stays_local_to_one_shot_request`，
  先红灯失败于缺少 `GuiFirstScreenSummaryTiming`，实现后通过。
- Phase 9u 已跑：`cargo test --manifest-path src-tauri/Cargo.toml first_screen_summary`，通过 2 个
  Tauri crate 单元测试，覆盖 first-screen summary timing 默认 shape 和 recoverable error shape。
- Phase 9u 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_summary`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、33 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 2 个 first-screen summary tests。
- Phase 9v 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_shape_is_static_and_explicit` 先红灯失败于缺少
  `gui_first_screen_refresh_shape`，实现后通过；`cargo test --manifest-path src-tauri/Cargo.toml first_screen_refresh_shape`
  通过 1 个 Tauri crate 单元测试。
- Phase 9v 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_refresh_shape`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、34 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen refresh shape test。
- Phase 9w 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_readiness_shape_is_static_display_preflight`
  先红灯失败于缺少 `gui_first_screen_readiness_shape`，实现后通过；
  `cargo test --manifest-path src-tauri/Cargo.toml first_screen_readiness_shape` 通过 1 个 Tauri crate
  单元测试。
- Phase 9w 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_readiness_shape`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、35 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen readiness shape test。
- Phase 9x 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_offline_shape_is_static_display_preflight`
  先红灯失败于缺少 `gui_first_screen_offline_shape`，实现后通过；
  `cargo test --manifest-path src-tauri/Cargo.toml first_screen_offline_shape` 通过 1 个 Tauri crate
  单元测试。
- Phase 9x 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_offline_shape`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、36 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen offline shape test。
- Phase 9y 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_command_policy_shape_keeps_one_shots_explicit`
  先红灯失败于缺少 `gui_first_screen_command_policy_shape`，实现后通过；
  `cargo test --manifest-path src-tauri/Cargo.toml first_screen_command_policy` 通过 1 个 Tauri crate
  单元测试。
- Phase 9y 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_command_policy`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、37 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen command policy test。
- Phase 9z 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_affordance_shape_stays_static`
  先红灯失败于缺少 `gui_first_screen_refresh_affordance_shape`，实现后通过；
  `cargo test --manifest-path src-tauri/Cargo.toml first_screen_refresh_affordance` 通过 1 个
  Tauri crate 单元测试。
- Phase 9z 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml first_screen_refresh_affordance`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、38 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate 单测覆盖 1 个 first-screen refresh affordance shape test。
- Phase 9aa 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_click_wiring_is_explicit_only`
  先红灯失败于缺少 `refresh-action-button`，实现后通过。
- Phase 9aa 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、39 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ab 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_result_projection_stays_click_scoped`
  先红灯失败于缺少 `projectExplicitRefreshSummary`，实现后通过。
- Phase 9ab 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、40 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ac 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_error_projection_stays_catch_scoped`
  先红灯失败于缺少 `projectExplicitRefreshError`，实现后通过。
- Phase 9ac 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、41 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ad 已跑窄验证：
  `cargo test --test platform_layout gui_first_screen_refresh_success_clears_offline_display`
  先红灯失败于 success projection 未清理 `offline-problem-kind`，实现后通过。
- Phase 9ad 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、42 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ae 已跑窄验证：
  `cargo test --test platform_layout gui_frontend_invokes_are_authorized_and_init_errors_are_visible`
  先红灯失败于 `allow-gui-shell-metadata` 未授权；补 `src-tauri/permissions/gui.toml`、capability
  allow 列表和初始化错误投影后通过。
- Phase 9ae 已跑 Tauri 验证：`cargo check --manifest-path src-tauri/Cargo.toml`，先红灯失败于
  application permission 文件缺失，补 `src-tauri/permissions/gui.toml` 后通过。
- Phase 9ae 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、43 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9af 已跑窄验证：
  `cargo test --test platform_layout gui_static_frontend_global_tauri_api_is_enabled_and_missing_api_is_visible`
  先红灯失败于 `src-tauri/tauri.conf.json` 未启用 `withGlobalTauri`；补配置和 missing API 错误显示后通过。
- Phase 9af 已跑 Tauri 验证：`cargo check --manifest-path src-tauri/Cargo.toml`，通过。
- Phase 9af 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、44 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ag 已跑窄验证：
  `cargo test --test platform_layout gui_manual_refresh_summary_is_readable_and_click_scoped`
  先红灯失败于缺少 `manual-summary-status`；补静态 summary 字段和 success/error projection 后通过。
- Phase 9ag 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、45 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ah 已跑窄验证：
  `cargo test --test platform_layout gui_frontend_first_screen_view_model_is_local_preflight_only`
  先红灯失败于缺少 `firstScreenViewModel`；补本地 view model 和 projection helper 后通过。
- Phase 9ah 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、46 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ai 已跑窄验证：
  `cargo test --test platform_layout gui_backend_event_stream_start_is_tauri_owned_and_explicit`
  先红灯失败于缺少 backend event stream command；补 Tauri-owned explicit stream command 后通过。
- Phase 9ai 已跑 Tauri 验证：`cargo check --manifest-path src-tauri/Cargo.toml`，通过。
- Phase 9ai 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、47 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9aj 已跑窄验证：
  `cargo test --test platform_layout gui_frontend_daemon_event_listener_wiring_is_event_only`
  先红灯失败于缺少 `window.__TAURI__.event.listen`；补 frontend listener、stream start 和 event projection 后通过。
- Phase 9aj 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、48 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9ak 已跑窄验证：
  `cargo test --test platform_layout gui_backend_event_stream_forwards_recording_state_changes`
  先红灯失败于缺少 `Event::StateChanged` mapper；补 mapper 后用户验证仍失败；强化测试要求 stream
  loop 不再用 shared first-screen classifier 过滤，改由 `gui_daemon_event_payload()` 直接决定 emit 后通过。
- Phase 9ak 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、49 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 9al 已跑窄验证：
  `cargo test --test platform_layout gui_event_stream_projects_first_screen_data_without_refresh`
  先红灯失败于缺少 live stats/text/history appended projection；补 backend payload 和 frontend projection 后通过。
- Phase 9al 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --manifest-path src-tauri/Cargo.toml`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、50 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests；
  Tauri crate `cargo check` 通过。
- Phase 7b/8b 已跑窄验证：
  `cargo test --test platform_layout overlay_windows_linux_backend_skeletons_are_cfg_gated_and_gui_free`
  先红灯失败于缺少 `src/overlay/windows.rs`，补 Windows/Linux cfg-gated backend skeleton 后通过。
- Phase 7b/8b 已跑：`cargo test overlay::renderer::tests`，通过 3 个 renderer 单元测试。
- Phase 7b/8b 已跑 cross target check：
  `cargo check --target x86_64-pc-windows-msvc` 被既有 Unix-only `src/ipc/transport.rs` 阻断；
  `cargo check --target x86_64-unknown-linux-gnu` 被 OpenSSL cross sysroot 阻断。这不是 overlay
  skeleton 自身的完整非 macOS 编译证明，需后续 IPC transport / Linux build 环境阶段解决。
- Phase 7b/8b 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、51 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 3b IPC transport cfg boundary 已跑窄验证：
  `cargo test --test platform_layout ipc_transport_backends_are_cfg_gated` 先红灯失败于 transport 未 cfg-gate，
  补 `src/ipc/transport.rs` Unix/Windows backend skeleton 后通过。
- Phase 3b 已跑：`cargo test ipc::transport::tests`，通过 3 个 Unix UDS transport 测试。
- Phase 3b 已跑：`cargo test platform::lifecycle`，通过 2 个 Unix lifecycle 测试。
- Phase 3b 已跑：`cargo check --target x86_64-pc-windows-msvc`，exit 0；仍有大量 dead-code/unused
  warning，原因是 Windows backend 多数仍是 unsupported skeleton，后续不能把它等同于 Windows 可运行。
- Phase 3b 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、639 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency`
  tests、52 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 10a 已跑：`cargo test cli::doctor::tests`，通过。
- Phase 10a 已跑：`cargo test --test platform_layout network_clients_use_rustls_for_cross_platform_checks`，
  通过。
- Phase 10a 已跑：`cargo fmt --check`，通过。
- Phase 10a 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 10a 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  640 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、53 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 10a 已跑：`make check-windows`，exit 0；仍有大量 dead-code/unused warning，原因是
  Windows backend 多数仍是 unsupported skeleton，不能等同于 Windows 可运行。
- Phase 10a 已跑：`make check-linux`，失败于缺少 `x86_64-linux-gnu-gcc` / Linux sysroot；
  已越过 OpenSSL/native-tls 阻断，当前是本机 cross toolchain 环境问题。
- Phase 10b 已跑窄验证：
  `cargo test tui::status::tests::platform_capability_lines_include_problem_details` 先红灯失败于缺少
  `platform_capability_lines`，实现后通过。
- Phase 10b 已跑：`cargo test --test platform_layout`，通过 54 个测试。
- Phase 10b 已跑：`cargo fmt --check`，通过。
- Phase 10b 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 10b 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  641 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、54 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 3c 已跑窄验证：
  `cargo test --test platform_layout windows_ipc_transport_uses_tokio_named_pipe_backend` 先红灯失败于
  Windows IPC transport 仍是 placeholder，改为 Tokio Named Pipe backend 后通过。
- Phase 3c 已跑：`cargo fmt --check`，通过。
- Phase 3c 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 3c 已跑：`cargo test --test platform_layout`，通过 55 个测试。
- Phase 3c 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  641 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、55 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 3c 已跑：`make check-windows`，exit 0；仍有大量 dead-code/unused warning，原因是
  Windows hotkey/overlay/service/lifecycle 等 backend 仍多为 skeleton，不能等同于 Windows runtime 可用。
- Windows IPC capability sync 已跑窄验证：
  `cargo test --test platform_layout windows_capability_snapshot_marks_named_pipe_transport_partial`
  先红灯失败于缺少 Windows capability override，实现后通过。
- Windows IPC capability sync 已跑：`cargo fmt --check`，通过。
- Windows IPC capability sync 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Windows IPC capability sync 已跑：`cargo test --test platform_layout`，通过 56 个测试。
- Windows IPC capability sync 已跑：`make check-windows`，exit 0；仍有 skeleton warning。
- Windows IPC capability sync 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  641 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、56 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 10c 已跑环境探测：
  `cross --version` 为 `cross 0.2.5`；`docker info --format '{{.ServerVersion}} {{.OSType}}/{{.Architecture}}'`
  为 `27.5.1 linux/aarch64`；`rustup target list --installed` 包含
  `aarch64-apple-darwin`、`x86_64-unknown-linux-gnu`。
- Phase 10c 已跑：`cross check --target x86_64-unknown-linux-gnu`，失败于
  `toolchain 'stable-x86_64-unknown-linux-gnu' may not be able to run on this system`，尚未进入 Docker
  编译。
- Phase 10c 已尝试：
  `rustup toolchain add stable-x86_64-unknown-linux-gnu --profile minimal --force-non-host`，开始下载
  3 个组件但第一次超过 90 秒无新输出后中断；第二次恢复半安装并完成。
- Phase 10c 已跑：`DOCKER_DEFAULT_PLATFORM=linux/amd64 docker pull ghcr.io/cross-rs/x86_64-unknown-linux-gnu:0.2.5`
  并用 `docker run ... uname -m` 验证容器为 `x86_64`。不设置该变量时 Apple Silicon Docker 会失败于
  `no matching manifest for linux/arm64/v8`。
- Phase 10c 已跑：
  `DOCKER_DEFAULT_PLATFORM=linux/amd64 cross check --target x86_64-unknown-linux-gnu`，进入 Docker
  编译后失败于 `openssl-sys` 找不到 OpenSSL；`cargo tree --target all -i openssl-sys` 显示真实来源是
  `voice_activity_detector` 默认启用 `ort/download-binaries`，进而由 `ort-sys` build dependency
  `ureq/native-tls` 拉入 OpenSSL。
- Phase 10c 已跑：曾尝试新增 `Cross.toml` 安装 `pkg-config libssl-dev`，`make check-linux-cross`
  进入 custom image build，但 apt
  失败于容器内 `127.0.0.1:7890` 代理不可达。宿主机 `127.0.0.1:7890` 和 Docker 内
  `host.docker.internal:7890` 均可连；已在 Makefile 目标中覆盖 HTTP/HTTPS proxy 到
  `host.docker.internal:7890`，但外层环境没有传进 Dockerfile build step；继续把代理覆盖写入
  `Cross.toml` pre-build 后 apt 可安装，但 Ubuntu xenial 的 OpenSSL 1.0.2 不满足
  `openssl-sys 0.9.116` 的 OpenSSL 1.1.0+ 要求。因此撤销安装 OpenSSL 的方案，改为 Linux target
  不依赖 `voice_activity_detector`，用 Silero unavailable stub 避开 build-time download/native-tls
  链路。
- Phase 10c 已跑：`cargo tree --target x86_64-unknown-linux-gnu -i openssl-sys` 和
  `cargo tree --target x86_64-unknown-linux-gnu -i voice_activity_detector` 在 Linux target 下均
  `warning: nothing to print`；随后 `make check-linux-cross` 继续推进到 `alsa-sys`，失败于缺少
  `alsa.pc` / `libasound2-dev`。已新增 `Cross.toml` 只安装 `pkg-config libasound2-dev`，尚需重新验证。
- Phase 10c 已跑：`cargo test --test platform_layout linux_cross_check_does_not_download_vad_runtime_at_build_time`
  先红灯失败于 Linux 仍直接依赖 `voice_activity_detector`，改成 Linux Silero unavailable stub 后通过。
- Phase 10c 已跑：`make check-linux-cross`，exit 0。首次构建会创建
  `cross-custom-shuohua:x86_64-unknown-linux-gnu-a42a7-pre-build`，并有大量非 macOS skeleton/dead-code
  warnings；这些 warnings 不等同于 Linux runtime 可用。
- Phase 10c 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、641 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency` tests、
  57 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 10d 已跑窄验证：
  `cargo test --test platform_layout linux_capability_snapshot_marks_compile_checked_unix_primitives`
  先红灯失败于缺少 `fn linux_capabilities()`，实现 Linux capability override 后通过。
- Phase 10d 已跑：`cargo test platform::capability::tests`，macOS target 下 6 个 capability tests 通过。
- Phase 10d 已跑：`make check-linux-cross`，exit 0；仍有非 macOS skeleton/dead-code warnings。
- Phase 10e 已跑窄验证：
  `cargo test --test platform_layout linux_service_manager_has_systemd_user_dry_run_skeleton`，通过。
- Phase 10e 已跑：`cargo test platform::service::imp::tests`，macOS target 下 12 个 launchd service
  tests 通过。
- Phase 10e 已跑：`cargo fmt --check`，通过。
- Phase 10e 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 10e 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  641 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、59 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 10e 已跑：`make check-linux-cross`，exit 0；仍有非 macOS skeleton/dead-code warnings，
  但未声明 Linux runtime 可用。
- Phase 10f 已跑窄验证：
  `cargo test --test platform_layout linux_service_manager_capability_reports_dry_run_skeleton`
  先红灯失败于缺少 `systemd_user_dry_run`，实现后通过。
- Phase 10f 已跑：`cargo test --test platform_layout linux_capability_snapshot_marks_compile_checked_unix_primitives`，
  通过。
- Phase 10f 已跑：`cargo test platform::capability::tests`，macOS target 下 6 个 capability tests 通过。
- Phase 10f 已跑：`cargo fmt --check`，通过。
- Phase 10f 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 10f 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  641 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、60 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 10f 已跑：`make check-linux-cross`，exit 0；仍有非 macOS skeleton/dead-code warnings，
  但未声明 Linux runtime 可用。
- Phase 10g 已跑新增测试红灯：
  `cargo test --test platform_layout path_open_reveal_lives_behind_platform_facade`
  先失败于缺少 `src/platform/path.rs`；实现 facade 后通过。
- Phase 10g 已跑新增测试红灯：
  `cargo test --test platform_layout linux_path_open_reveal_capability_reports_xdg_open_partial`
  先失败于缺少 Linux `xdg_open` capability token；实现后通过。
- Phase 10g 已跑：`cargo test tui::audio::tests`，通过 11 个测试。
- Phase 10g 已跑：`cargo test tui::config_actions::tests`，通过 5 个测试。
- Phase 10g 已跑：`cargo fmt --check`，通过。
- Phase 10g 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- Phase 10g 已跑：`cargo test`，通过。`cargo test` 覆盖：92 个 library unit tests、
  641 个 binary unit tests、5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、
  2 个 `doc_consistency` tests、62 个 `platform_layout` tests、6 个 `theme_registry_build` tests、
  0 个 doctests。
- Phase 10g 已跑：`make check-linux-cross`，exit 0；仍有非 macOS skeleton/dead-code warnings，
  但未声明 Linux desktop runtime 可用。
- Phase 10h 已跑窄验证：
  `cargo test --test platform_layout path_open_reveal_lives_behind_platform_facade`，通过。
- Phase 10h 已跑窄验证：
  `cargo test --test platform_layout windows_path_open_reveal_capability_reports_explorer_partial`，通过。
- Phase 10h 已跑：`make check-windows`，exit 0；仍有非 macOS skeleton/dead-code warnings，
  但 `platform::path` 的 Windows backend 编译通过。
- Phase 10h 已跑：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`，
  通过。`cargo test` 覆盖：92 个 library unit tests、641 个 binary unit tests、
  5 个 `apple_helper_build` tests、1 个 `cli_runtime_boundary` test、2 个 `doc_consistency` tests、
  63 个 `platform_layout` tests、6 个 `theme_registry_build` tests、0 个 doctests。
- Phase 10h 已跑：`make check-linux-cross`，exit 0；仍有非 macOS skeleton/dead-code warnings。
- macOS 权限、录音、overlay、clipboard/paste、TUI、service lifecycle、history 手动体验：未执行，
  需用户在真实 macOS 会话按 `macos-baseline.md` checklist 验证。

## 已知风险

- `src/cli/doctor.rs` 仍有 launchd-centric 诊断输出；service manager facade 后应通过
  capability/status 和 service manager 模型收敛。
- Phase 5b 只抽 hotkey provider 启动边界，没有实现 Linux/Windows global hotkey backend。
- Phase 10b 只把 renderer/platform capability snapshot 接入 TUI Status 静态摘要；Phase 7b/8b
  已有 Windows/Linux overlay backend skeleton，但还没有真实 renderer 实现。
- Phase 7a 只是 Microsoft 文档基线，不代表已在 Windows 11/10 真机验证。实际 topmost、
  no-activate、click-through、材质、capture exclusion 和性能数据仍需 PoC 记录。
- Phase 8a 只是 Wayland/layer-shell 文档基线，不代表已在 wlroots/KDE/GNOME 真机验证。
  实际 layer-shell availability、top layer、pointer passthrough、alpha、screen anchor 和性能
  数据仍需 PoC 记录。
- Phase 9a 只是 Tauri v2 文档基线，不代表已测 GUI 冷启动、内存、CPU、包体或三端打包。
  GUI PoC 仍需证明 daemon 未打开 GUI 时不加载 WebView，且 GUI 退出不影响 daemon。
- Phase 9c 只提供首屏 command helper 和 event classifier；尚未实现真实 Tauri GUI app、
  frontend view model、重连策略、指标采集或打包验证。
- Phase 9f 已创建最小 library target，但 surface 仍包含现有 `history` / `state` 模型，而不是
  更小 wire DTO；这避免协议复制，但也意味着 GUI backend 会看到这些数据模型。
- Phase 9g 只提供连接状态/退避骨架，没有实现真实后台 reconnect task、Tauri event bridge
  或 daemon offline UI view model。
- Phase 9h 只提供 GUI backend event bridge 的纯封装，没有实现 Tauri event emission、
  frontend view model 或后台 reconnect loop。
- Phase 9i 只提供首屏 metrics/timing 纯模型，没有实现真实 metrics sink、Tauri event
  emission、前端展示、后台 reconnect loop 或打包指标采集。
- Phase 9j 只记录 Tauri permissions/capabilities preflight，没有创建真实 Tauri workspace、
  capabilities JSON、frontend command binding 或打包验证。
- Phase 9k 只记录创建 Tauri workspace 前的验收清单，没有创建真实 Tauri workspace、
  capabilities JSON、frontend command binding、release build 或打包验证。
- Phase 9l 只记录 reconnect supervisor ownership/cancellation 语义，没有实现真实 runtime loop、
  Tauri event emission、frontend view model 或 metrics sink。
- Phase 9m/9n/9o/9p/9q/9r/9s/9t/9u/9v/9w/9x/9y/9z/9aa/9ab/9ac/9ad/9ae/9af/9ag/9ah/9ai/9aj/9ak/9al 只创建最小 `src-tauri/**` skeleton、静态 placeholder、本地 metadata
  command、first-screen request plan command、daemon status snapshot shape command、纯 daemon
  status event mapper、显式 one-shot daemon status request command 和显式 one-shot history summary
  request command、显式 one-shot first-screen summary request command、first-screen summary 本地
  timing 字段、first-screen explicit refresh shape、first-screen readiness/timing display shape 和
  first-screen offline/error display shape、first-screen command invocation policy shape、
  first-screen explicit refresh affordance shape、placeholder explicit refresh click wiring 和
  click-scoped summary/error projection、success offline clear、application command ACL、初始化错误可见性和静态
  frontend global Tauri API、手动 Refresh 可读摘要、本地 first-screen view model 和显式 backend
  daemon event stream bridge、frontend daemon event listener wiring、`StateChanged` forwarding 和
  first-screen stream data projection；
  尚未运行 `tauri dev` / `tauri build` / `tauri bundle`，也没有启动 GUI 或 daemon。后续需要
  单独决定何时运行 release build、如何记录 cold start/RSS/CPU/bundle 指标。
- Phase 9n 的 `gui_shell_metadata` 只验证本地 command wiring，不连接 daemon、不读
  config/history、不生成真实 Status/History/Diagnostics view model。
- Phase 9o 的 `gui_first_screen_request_plan` 只生成请求计划 summary，不发送 IPC、不订阅
  event stream、不读取 daemon status。
- Phase 9p 的 `gui_daemon_status_snapshot` 只固定 status response shape，不连接 daemon、
  不发送 `Command::DaemonStatus`、不读取真实 `Event::DaemonStatus`。
- Phase 9q 的 `gui_daemon_status_snapshot_from_event` 只映射调用方已提供的
  `Event::DaemonStatus`；仍没有真实 IPC request、event stream 或 Tauri event emission。
- Phase 9r 的 `gui_daemon_status_request_once` 只做显式 one-shot request；不自动调用、不订阅、
  不重连、不启动 daemon、不提供 service management。
- Phase 9s 的 `gui_history_summary_request_once` 只做显式 one-shot request；不自动调用、不订阅、
  不重连、不启动 daemon、不提供完整 History view model。
- Phase 9t 的 `gui_first_screen_summary_request_once` 只做显式 one-shot request；不自动调用、
  不订阅、不重连、不启动 daemon、不提供 frontend Status/History view model。
- Phase 9u 的 first-screen summary timing 只描述本次显式 request 的 GUI backend 本地耗时；
  不代表 daemon 内部状态，不写入 protocol/history/trace。
- Phase 9v 的 `gui_first_screen_refresh_shape` 只描述后续前端手动刷新入口的静态 shape；
  placeholder 不自动调用 `gui_first_screen_summary_request_once`，也不实现 loading/retry UI。
- Phase 9w 的 `gui_first_screen_readiness_shape` 只描述 placeholder 首屏 readiness/timing 空态；
  不读取真实 daemon event、不调用 one-shot request、不启动 timer 或 metrics sink。
- Phase 9x 的 `gui_first_screen_offline_shape` 只描述 placeholder 首屏 daemon offline /
  recoverable error 空态；不启动 daemon、不安装/重启 service、不启动 reconnect loop。
- Phase 9y 的 `gui_first_screen_command_policy_shape` 只描述 placeholder 可自动调用的静态
  command 和必须显式触发的 one-shot command；不作为真实 command dispatcher。
- Phase 9z 的 `gui_first_screen_refresh_affordance_shape` 只描述 placeholder 手动刷新控件的
  静态展示字段；不注册真实 click handler，不自动调用 one-shot request。
- Phase 9aa 的 placeholder refresh button 只在用户 click 后调用既有
  `gui_first_screen_summary_request_once`；初始加载不自动请求，不订阅、不重连、不启动 daemon。
- Phase 9ab 的 `projectExplicitRefreshSummary` 只在 explicit refresh click 成功路径内把 summary
  投影到现有 placeholder 文本字段；不新增 backend command，不建立完整 view model。
- Phase 9ac 的 `projectExplicitRefreshError` 只在 explicit refresh click catch 路径内把 request
  error 投影到现有 placeholder 文本字段；不新增 backend command，不实现 retry loop。
- Phase 9ad 的 `projectExplicitRefreshSummary` 只在 explicit refresh click success 路径内清理
  stale offline/error 文本；不新增 backend command，不新增请求。
- Phase 9ae 只保证当前 placeholder frontend invoke 的 Tauri application commands 被 capability
  授权，并且初始化失败不再静默吞掉；不实现 daemon event subscription、recording state streaming、
  reconnect supervisor 或自动首屏 one-shot。
- Phase 9af 只为无 bundler 静态 HTML 启用 `withGlobalTauri`，并在 `window.__TAURI__` API 缺失时显示
  `tauri-api-missing`；不实现 daemon event subscription、recording state streaming、reconnect
  supervisor 或自动首屏 one-shot。
- Phase 9ag 只在 explicit Refresh success/catch 路径更新 manual summary 文本；不实现 daemon event
  subscription、recording state streaming、reconnect supervisor 或自动首屏 one-shot。
- Phase 9ah 只在静态 HTML 内维护本地 `firstScreenViewModel`；不实现 daemon event subscription、
  recording state streaming、reconnect supervisor 或自动首屏 one-shot。
- Phase 9ai 只在 Tauri backend 暴露显式 `gui_start_daemon_event_stream` command 并启动
  GUI-owned event stream task；不实现 reconnect supervisor、daemon auto-start 或 service management。
- Phase 9aj 只在 frontend 初始化时注册 Tauri event listener 并显式启动 event stream bridge；
  event payload 只投影到 placeholder view model/DOM，不提供 start/stop/cancel recording controls、
  reconnect supervisor、window close cancellation 或完整 Status/History view。
- Phase 9ak 只修复 backend stream mapper，把既有 `StateChanged` 转成现有 `daemonStatus`
  payload，并移除 stream loop 对 shared first-screen classifier 的前置过滤；不新增 IPC event、不改变
  daemon/TUI 行为、不新增 GUI recording controls。
- Phase 9al 只把既有 `StatsChanged`、`Partial`、`Segment`、`HistoryAppended` 投影到现有
  placeholder 字段；不自动触发 Refresh、不建立完整 History view、不新增 IPC event 或 polling。
- GUI PoC 冻结：`src-tauri/**` 和 `gui-dist/index.html` 只保留为未来 GUI 接口验证成果；不要继续
  打磨 placeholder 页面，不实现 reconnect supervisor、recording controls、service management、
  配置编辑器或 release/bundle 指标，除非重新进入 GUI 产品设计阶段。
- Phase 7b/8b overlay skeleton 已开始：`src/overlay/windows.rs` 和 `src/overlay/linux.rs`
  作为 cfg-gated backend skeleton，`overlay::renderer` 在 Windows/Linux 下调度到对应 backend。
  Windows 当前报告 `win32_overlay_skeleton` structured unsupported；Linux 当前报告
  `wayland_overlay_skeleton`，其中 window anchor 为 `degraded/screen_anchor_expected`。
- `ipc::transport` 已有 Windows Named Pipe compile backend，但未在 Windows 实机/VM 验证 runtime
  connect/bind/accept、ACL/security descriptor、multi-user 隔离或 pipe busy 行为。Windows daemon
  lock/process probe/smart fallback 同样仍只是 unsupported skeleton。
- `current_platform_capabilities()` 是 Phase 1 静态快照，不执行权限 probe；后续消费方不要把
  静态 `desktop.permissions=available` 误解为当前已授权。
- `overlay::renderer::renderer_capabilities()` 同样是静态快照，不创建窗口、不 probe 当前
  compositor/权限、不读取业务配置。

## 下一步

最新验证结果：

- Windows native:
  - `cargo fmt --check` 通过。
  - `cargo clippy --all-targets -- -D warnings` 通过。
  - `cargo test` 通过。
  - `cargo test --target x86_64-pc-windows-msvc` 通过。
  - `cargo build --target x86_64-pc-windows-msvc` 通过。
- Windows runtime smoke:
  - `shuo.exe --version` 通过。
  - `shuo.exe doctor` 能运行并使用 `%APPDATA%\Shuohua`，但因本机配置/设备/权限返回 1。
  - `shuo.exe service status` 在 daemon running/not running 两种状态下均通过，且只做 dry-run/status。
  - `shuo.exe --daemon` + scoped Named Pipe `DaemonStatus` 通过。
  - 第二个 `shuo.exe --daemon` 明确失败，单实例 guard 生效。
  - Explorer direct open/reveal 不挂起，但工具会话返回 1，窗口行为未人工确认。
- 未在本 Windows session 跑 `make check-windows` / `make check-linux-cross`；本阶段验证重点是
  Windows native build/test/runtime smoke。

下一步：

- Phase 10y 或手动停点：优先做 cross-user 第二账号/VM 隔离验证；没有第二用户前不要升级
  Windows IPC capability。
- 后续代码小步可选：在不改变 capability 结论的前提下，设计 raw `CreateFileW`/overlapped client
  access-mask narrowing，或把 busy/elevation smoke 脚本沉淀为开发者手动命令。
- audio、overlay、hotkey、clipboard/paste 都必须在 Windows runtime 上手动验证后才允许 capability
  升级。
- 不继续 GUI 产品化开发。

建议下一 session prompt：

```text
继续 /Users/ghot/repo/shuohua 跨平台改造，当前在 feat/cross-platform-design。
先读 AGENTS.md、TODO、docs/cross-platform/README.md、overview.md、
development-plan.md、gui.md、overlay.md、platform-capabilities.md、macos-baseline.md、
handoff.md。
Phase 9al 后 GUI PoC 已冻结；不要继续打磨 GUI placeholder。
Phase 10m Windows Development Design Baseline 已完成；`docs/cross-platform/windows.md` 是
Windows-first 实现基线。Phase 10m1 App Data Ownership Baseline 已完成；
`docs/cross-platform/app-data.md` 规定 CLI/daemon/GUI/packaged app 共享 product data root，
package/app-private data 只放 GUI/runtime 私有状态。Phase 10n Windows runtime validation checklist、
Phase 10o Windows path/config/state backend、Phase 10p Windows local development setup 已完成。
下一步是 Phase 10q Windows Named Pipe endpoint scoping/security descriptor 和 runtime smoke。
Phase 7b/8b overlay backend skeleton、Phase 3b IPC transport cfg boundary、Phase 10a
cross-check baseline、Phase 10b TUI capability diagnostics、Phase 10c Docker/cross Linux
check baseline、Phase 10d Linux compile-time capability sync、Phase 3c Windows Named Pipe
transport compile backend、Windows IPC capability sync、Phase 10i audio convert facade、
Phase 10j Windows lifecycle primitive compile backend、Phase 10k Windows service dry-run/status
skeleton 和 Phase 10l non-macOS desktop capability truthfulness 已完成。先查看最新
diff/commit 和验证结果。
保持 macOS 不回退，不引入 GUI/WebView。不要把 Windows Named Pipe compile backend 当成实机
runtime 验收。不要添加 GitHub Actions Windows artifact job；Windows 机器作为本地开发/build/runtime
测试环境，通过 GitHub 同步代码。真实 Windows IPC/audio/overlay/hotkey/clipboard 验证需要用户目标系统。
```
