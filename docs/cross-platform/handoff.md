# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近阶段 commit

Latest phase commit: `feat: add vad probability hysteresis`（本阶段提交；以 `git log -1` 为准）。

Previous phase commit: `feat: encode windows lossless audio natively` (`f8c97ac`).

Note: handoff-only sync commits may be newer than the latest phase commit; use `git log -1` for the exact
current HEAD.

当前分支已 rebase 到 `v0.2.0` / `release: v0.2.0` 基底（commit `7fff199`）。

## 当前 phase

GUI PoC 冻结，当前主线切到 Windows-first core runtime。

Phase 10bx Windows paste target smoke 已完成：

- 新增 Windows ignored runtime smoke
  `platform::windows::autotype::tests::paste_into_win32_edit_runtime_smoke`，它把 Unicode 文本写入 Win32
  clipboard，创建 foreground Win32 `EDIT` control，调用真实 `platform::autotype::paste()` / `SendInput`
  Ctrl+V，再用 `GetWindowTextW` 读回目标控件文本。
- 本机已通过：
  `SHUOHUA_WINDOWS_PASTE_TARGET_SMOKE_TEXT='shuohua paste target smoke 20260627 字🙂' cargo test --target
  x86_64-pc-windows-msvc platform::windows::autotype::tests::paste_into_win32_edit_runtime_smoke -- --ignored
  --exact`。
- Windows `desktop.text_injection` capability 仍为 `partial/sendinput_ctrl_v`，reason 从
  `runtime_smoke_only` 收窄为 `win32_edit_target_runtime_smoke`。这只证明 Ctrl+V 能进入测试进程创建的
  Win32 EDIT control，不覆盖 Notepad/browser/editor/terminal、IME、remote desktop、UAC/elevation 或 full
  record -> paste session。
- 验证已通过：`cargo fmt --check`、
  `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings`、
  `cargo test --target x86_64-pc-windows-msvc`、
  `cargo build --target x86_64-pc-windows-msvc`、`cargo test`。

Phase 10bw Windows dispatch clipboard smoke 已完成：

- 新增 Windows ignored runtime smoke
  `voice::dispatch::tests::windows_dispatch_clipboard_runtime_smoke`，直接调用
  `voice::dispatch::dispatch(text, false)`，再用 Win32 `GetClipboardData(CF_UNICODETEXT)` 读回同一 Unicode
  文本。
- 本机已通过：
  `SHUOHUA_WINDOWS_DISPATCH_SMOKE_TEXT='shuohua dispatch smoke 20260627 字🙂' cargo test --target
  x86_64-pc-windows-msvc voice::dispatch::tests::windows_dispatch_clipboard_runtime_smoke -- --ignored --exact`。
- Windows `desktop.clipboard` capability 仍为 `partial/win32_clipboard_unicode`，reason 从
  `write_only_runtime_smoke` 收窄为 `dispatch_clipboard_runtime_smoke`。这覆盖 voice dispatch -> platform
  desktop facade -> Win32 clipboard backend 的真实路径，但不触发 `SendInput`、不验证目标 App paste、
  elevation/UAC 或 full record -> paste。
- 验证还需跑 Windows target fmt/test/build/clippy；本阶段不涉及 overlay、hotkey 或 audio 行为。

Phase 10bv Windows full recording audio capability sync 已完成：

- 用户手动验证 Phase 10bu 后反馈 VAD 体感“没有什么问题”，因此当前 Windows Silero/VadPause 可继续作为
  非 overlay 能力闭环的可用 baseline；后续若要改变“思考多久进入 Idle”，应进入 endpoint policy /
  sensitivity 设计，而不是继续堆隐藏 Windows 参数。
- Windows `audio.capture` capability 仍为 `partial/cpal_wasapi`，reason 从 `input_stream_runtime_smoke`
  收窄为 `full_recording_history_smoke`，因为真实麦克风 recording 已产生 `submitted` history 和 retained
  audio。
- Windows `audio.convert` capability 仍为 `partial/media_foundation_aac_flacenc`，reason 从
  `native_conversion_runtime_smoke` 收窄为 `full_recording_history_smoke`，因为 compact `.m4a` 和 lossless
  `.flac` 都已在 full recording 下生成并关联 history。
- 仍不能升级到 `available`：Explorer open/reveal、音频播放、长时间 recording soak、多设备/权限矩阵和
  remote desktop 边界尚未覆盖。
- 验证：本阶段属于 capability/docs sync，需跑 `cargo fmt --check` 和
  `cargo test --target x86_64-pc-windows-msvc platform_layout::windows_audio_capture_capability_reports_input_stream_smoke
  platform_layout::windows_audio_convert_capability_reports_native_compact_backend` 或完整 Windows target tests。

Phase 10bu VAD probability hysteresis 已完成：

- 用户完成 Windows native retained-audio 手动 smoke；本 session 复核
  `%LOCALAPPDATA%\Shuohua\audio\01KW3RWQJ0YM9CRBT228BX46TM.m4a` 和
  `%LOCALAPPDATA%\Shuohua\audio\01KW3RXZQ9ZVGJHJJKBH09PRMQ.flac` 存在，对应
  `%LOCALAPPDATA%\Shuohua\history\2026-06.jsonl` 记录均为 `submitted`、provider 为 `doubao`。
  该结论验证 compact/lossless native conversion 在真实 recording 下能产物并关联 history；未单独确认
  Explorer open/reveal/playback，因此 capability 仍保持 `partial`。
- 两段真实录音暴露 VadPause 会在用户中途停顿后进入 Idle。离线诊断显示当前配置
  `threshold=0.5`、`pause_silence_ms=1500`、`min_start_voiced_frames=2` 下，两段录音各产生 3 次
  resume/pause；这是端点策略对超过 1.5s 低概率停顿的正常响应，不是 retained audio 或 Silero init 问题。
- 本阶段新增通用 VAD probability hysteresis：Silence -> Speech 仍使用配置中的 `threshold`，Speech -> Silence
  使用派生的较低 exit threshold 统计静音，降低概率在阈值附近抖动导致的过早 pause。没有修改用户
  `%APPDATA%\Shuohua\config.toml`，也没有加入 Windows 隐式 `pause_silence_ms` 覆盖。
- engine 恢复 Recording 后不再 reset 再喂 synthetic `VadFrame::Speech`，改为 `reset_to_speech()` 明确进入
  active 状态，避免人为预置一帧 speech 影响后续计数。`diagnostics silero-vad-file` 和 dev trace 现在复用
  runtime 同一套 `VadController` 判定。
- 验证已通过：`cargo fmt --check`、`cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings`、
  `cargo test --target x86_64-pc-windows-msvc`、`cargo build --target x86_64-pc-windows-msvc`、`cargo test`。
- 已知风险：这次修正不把长停顿强行合并成同一段；若产品希望“思考 2-3 秒仍保持 Recording”，下一步应调整
  用户可理解的 VAD sensitivity/endpoint policy，而不是继续暴露更多底层 frame 参数。

Phase 10bt Windows native retained-audio conversion 正在收尾：

- Windows `record_audio = "lossless"` 不再依赖 PATH 中的 `ffmpeg.exe`，改用 pure Rust `flacenc` 将
  recorder WAV 转成 FLAC。`flacenc` 已用 `--no-default-features` 接入，scratch Windows/MSVC 构建和本机
  ignored runtime smoke 都能生成 FLAC。
- Windows `record_audio = "compact"` 继续使用上一阶段 Windows Media Foundation Sink Writer 转 AAC/M4A
  32 kbps。
- `audio.convert` capability 仍保持 `partial`，但 backend/reason 更新为
  `media_foundation_aac_flacenc` / `native_conversion_runtime_smoke`。这表示 conversion dependency 已满足
  当前单二进制策略，但 full recording -> history/open/playback 仍待验证。
- 未完成：真实 hotkey-triggered Windows recording 下分别验证 compact `.m4a` 和 lossless `.flac` 生成、
  history 关联、Explorer open/reveal 和播放；通过前不能把 `audio.convert` 升级为 available。

Phase 10bs Windows native compact retained-audio conversion 已完成：

- Windows `record_audio = "compact"` 不再依赖 PATH 中的 `ffmpeg.exe`，改用 Windows Media Foundation
  Sink Writer 将 recorder WAV 转成 AAC/M4A 32 kbps。该路径不新增第三方依赖，符合单 exe 用户体验方向。
- 新增 ignored runtime smoke
  `platform::audio_convert::imp::tests::media_foundation_runtime_smoke_creates_m4a_without_ffmpeg`，已在本机
  Windows 通过，证明 native compact `.m4a` 可生成且无需 ffmpeg。
- voice retained-audio finish smoke 拆成 native compact 与 external lossless 两条，避免把 compact 误写成
  ffmpeg 依赖。

Phase 10br Windows audio-processing dependency spike 已完成：

- 目标是评估是否应直接接入成熟 WebRTC Audio Processing Module，而不是继续手写 VAD gain 逻辑。
- 结论：WebRTC APM 仍是算法成熟度最高的长期方向，但当前 `webrtc-audio-processing 2.1.0` wrapper
  不能直接进入主线。默认构建依赖系统 `webrtc-audio-processing-2` 动态库，不满足三端单二进制分发；
  `bundled` 静态路径在 Windows/MSVC scratch build 中卡在 Unix 风格 build script/tooling。
- `sonora-agc2 0.1.0` 是 BSD-3-Clause、纯 Rust、Windows/MSVC scratch build 可过，但它只是 AGC2/RNN
  VAD 组件，不是完整 APM。若后续接入，应作为 `voice::preprocess` 的实验 backend，而不是改
  Silero/session orchestration。
- 本阶段不改变 runtime 行为、不新增依赖、不升级 capability。当前 Windows VAD 仍使用上一阶段
  `VadPreprocessor` baseline；用户手动 smoke 反馈当前效果“还不错”。
- 下一步建议：继续 Windows 非 overlay 能力闭环；若继续做音频处理，先做 packaging/license/build
  spike，不要直接把 APM dependency 放入主线。

Phase 10bq Windows VAD preprocessing baseline 已完成：

- Windows Silero VAD 现在通过 `voice::preprocess::VadPreprocessor` 处理 VAD-only PCM 副本；ASR PCM、
  retained audio、history 均不受影响。
- 当前 preprocessor 在 Windows 上做 RMS/peak gated adaptive gain，并平滑跨 frame 的 gain 变化；macOS
  当前仍 passthrough，避免破坏已可用行为。
- 曾尝试 Windows 隐式覆盖 `pause_silence_ms/min_start_voiced_frames`，已撤回；runtime 重新尊重
  config 中的 VAD 策略字段。`policy_from_config` 只是让 engine/diagnostics 共享同一条显式配置路径。
- 新增隐藏诊断命令 `shuo diagnostics silero-vad-file <path>`，用 ffmpeg 解码音频后输出 Silero 概率、
  effective policy 和 transition 时间点，方便不同设备/录音离线分析。
- 用户手动 smoke 反馈当前 Windows VAD 效果“还不错”。

Phase 10bp Windows Silero VAD parity 已完成 build/test 与单 exe init smoke：

- 用户明确产品目标：最终安装/分发体验应是用户拿到一个单 binary 即可运行，不需要手动安装 ORT/DLL/
  模型；内部是否依赖 Rust 以外 runtime 不重要。
- 因此废弃上一轮 `energy` 临时 backend 作为产品路径，Windows 直接对标 macOS 的 Silero 模型/API。
  `[voice.vad] backend` 继续只暴露 `off` / `silero`。
- Windows 不直接使用 crates.io 默认 `voice_activity_detector` ORT 装配；当前实现使用 renamed vendored
  `voice_activity_detector`，关闭 `ort` 默认特性并启用 `load-dynamic`。`shuo.exe` 内嵌官方 ONNX Runtime
  1.22.0 x64 DLL，启动 Silero 前释放到 `%LOCALAPPDATA%\Shuohua\cache\runtime\onnxruntime\...` 并显式
  `ort::init_from(path)`，避免链接失败和 `System32\onnxruntime.dll` 旧版本抢先加载。
- 本阶段新增隐藏诊断命令 `shuo diagnostics silero-vad`，用于不依赖麦克风地初始化 Silero、喂一帧静音、
  输出 frame/probability，作为 release 单 exe smoke 的二进制入口。
- Release 单 exe smoke 已通过：`cargo build --release --target x86_64-pc-windows-msvc` 后只复制
  `target\x86_64-pc-windows-msvc\release\shuo.exe` 到
  `%TEMP%\shuohua-single-exe-smoke`，运行 `--version`、`doctor`、`diagnostics silero-vad`。其中
  `diagnostics silero-vad` 输出 `silero-vad: OK frame=Silence probability=0.044263`，并确认释放出
  `%LOCALAPPDATA%\Shuohua\cache\runtime\onnxruntime\1.22.0\579b63640398\onnxruntime.dll`。
- `doctor` 在单 exe smoke 中可运行且未崩溃，但因当前用户配置
  `%APPDATA%\Shuohua\config.toml` 的 `voice.record_audio` 被写成无效值
  `compact测试一下压缩能不能实现？`，按预期返回配置错误。这是本机配置问题，不是 Silero/ORT
  provisioning 失败。
- 验证已通过：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、
  `cargo test --target x86_64-pc-windows-msvc`、`cargo build --target x86_64-pc-windows-msvc`、
  `cargo build --release --target x86_64-pc-windows-msvc`、clean-dir single exe
  `shuo.exe diagnostics silero-vad`。
- 未完成：真实 Windows 麦克风 VadPause 录音 smoke。通过前不升级 Windows VAD capability。

Phase 10bl Windows retained-audio IPC deletion smoke 已完成：

- Windows target 下新增 IPC server 层 retained-audio 删除测试，覆盖 `Command::DeleteAudio` 通过
  Named Pipe/IPC server 调用 `HistoryService::delete_audio` 并删除 fake `.m4a`。
- 既有 `delete_response_includes_record_deleted` 扩展为同时创建 fake `.flac`，验证
  `Command::DeleteHistory` 返回 `record_deleted=true`、`audio_deleted=true`、`audio_error=None`，且文件被删除。
- 该阶段不改变 runtime 行为、不升级 capability；只是补齐 retained audio 生成之后的维护路径自动化覆盖。
- 验证已通过：`cargo fmt --check`、
  `cargo test --target x86_64-pc-windows-msvc ipc::server::tests::delete_audio_command_removes_retained_audio_without_history_record -- --exact`、
  `cargo test --target x86_64-pc-windows-msvc ipc::server::tests::delete_response_includes_record_deleted -- --exact`。
- 下一步建议：继续非 overlay 能力闭环；可做 full Windows recording retained-audio manual smoke，或继续补
  history/audio open/reveal、route/clipboard/paste 的可自动 Windows guard。

Phase 10bm Windows full FLAC retained-audio recording smoke 已验证：

- 用户在 Windows 本机完成多段真实麦克风录音；本 session 复核 `%LOCALAPPDATA%\Shuohua\audio` 下存在
  `01KW2NRZ209KWYV6JFNCJK92HF.flac`、`01KW2NSYTQH9YT3KKYWFT9BQX7.flac`、
  `01KW2NTNGFQ360YZR8BXP9JDPE.flac`。
- 对应 history shard `%LOCALAPPDATA%\Shuohua\history\2026-06.jsonl` 中三条记录均为 `submitted`，
  provider 为 `doubao`，`audio_ms` 分别约 5.86s、21.43s、20.01s，文本内容与用户刚才录音相符。
- 当前生效配置仍是 `[voice] record_audio = "lossless"`，因此该 smoke 验证的是 full recording -> FLAC
  retained audio，不验证 compact/M4A full recording。
- Windows compact/M4A backend 仍已通过自动 ignored smoke：
  `platform::audio_convert::imp::tests::ffmpeg_runtime_smoke_creates_flac_and_m4a` 和
  `voice::audio::tests::ffmpeg_finish_creates_retained_audio_and_removes_temporary_wav`；full recording
  compact 模式还需要把 `record_audio = "compact"` 后再手动录一段确认 `.m4a`。
- VAD/Silero 仍未在 Windows 完成：Windows 当前因 ONNX Runtime provisioning 未定义而退回 Continuous
  recording；不要把 Windows VAD 或 `idle_pause` 视为完成。

Phase 10bn Windows full M4A retained-audio recording smoke 已验证：

- 用户在 Windows 本机把 `[voice] record_audio = "compact"` 后完成真实麦克风录音；本 session 复核
  `%LOCALAPPDATA%\Shuohua\audio\01KW2P08PZZYRW7DT2DXXBHVV2.m4a` 存在，大小约 20 KB。
- 对应 history shard `%LOCALAPPDATA%\Shuohua\history\2026-06.jsonl` 最新记录 id 为
  `01KW2P08PZZYRW7DT2DXXBHVV2`，status `submitted`，provider `doubao`，`audio_ms` 约 4.57s，
  文本为“测试一下压缩能不能实现？”。
- 至此 Windows full recording -> retained audio 的 `lossless`/FLAC 和 `compact`/M4A 路径均已通过真实
  手动 smoke；这仍不验证 VAD/Silero，也不升级 Windows VAD capability。

Phase 10bk Windows AppUserModelID active app identity 已完成：

- Windows `platform::desktop::frontmost_app()` 现在在 foreground window owner process 上同时尝试
  `QueryFullProcessImageNameW` 和 `GetApplicationUserModelId`。
- `AppContext.windows_exe_name` 继续作为 unpackaged Win32 fallback；`windows_app_user_model_id` 在 Windows
  能提供 AUMID 时填充。完整进程路径仍不进入 doctor/history/IPC。
- `desktop.active_app` capability 更新为
  `partial backend=foreground_window_process_identity reason=exe_name_and_optional_aumid`；AUMID 为空是正常
  降级，不能因此阻断 route fallback。
- 本机 `shuo.exe doctor` 通过，当前 Windows Terminal 前台输出：
  `exe_name=WindowsTerminal.exe app_user_model_id=Microsoft.WindowsTerminal_8wekyb3d8bbwe!App
  app_name=WindowsTerminal`，`profile.route.current` 仍命中 `agent`。
- 验证已通过：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、
  `cargo test --target x86_64-pc-windows-msvc`、`cargo build --target x86_64-pc-windows-msvc`、
  `cargo test --target x86_64-pc-windows-msvc platform::windows::app_context`、`shuo.exe doctor`。
- 下一步建议：继续非 overlay 能力闭环；可做 retained audio full recording smoke、TUI/history audio 操作
  Windows runtime smoke，或在真实目标 App 中验证 route/hotkey/clipboard/paste 矩阵。

Phase 10bj Windows optional retained-audio conversion 已完成：

- Windows `platform::audio_convert` 现在有可选外部 `ffmpeg` backend：`lossless` 转 FLAC，`compact` 转
  AAC/M4A 32 kbps。
- 本阶段不新增配置项，不打包/安装 ffmpeg；`ffmpeg.exe` 必须在 PATH 中。缺少 ffmpeg 时 retained audio
  save 失败并清理临时文件，文本 dispatch/history 不应依赖 retained audio 成功。
- `audio.convert` capability 从 Windows 默认 unsupported 收窄为
  `partial backend=ffmpeg_external reason=external_ffmpeg_optional`；在 full hotkey-triggered Windows
  recording 生成并回放 `.flac`/`.m4a` 前不能升级为 available。
- 本机 `where.exe ffmpeg` 命中 `C:\Users\hza2002\AppData\Local\Microsoft\WinGet\Links\ffmpeg.exe`，ignored
  runtime smoke 已验证 ffmpeg 能把测试 WAV 生成 FLAC 和 M4A。
- Windows retained-audio finish smoke
  `voice::audio::tests::ffmpeg_finish_creates_retained_audio_and_removes_temporary_wav` 已通过，覆盖
  `prepare -> tmp.wav -> finish -> final .flac/.m4a -> temp cleanup` 路径。
- 顺手修正了 overlay 文档/guard 的陈旧描述：当前 DirectWrite path 使用 `Microsoft YaHei UI` 和
  `ID2D1DCRenderTarget`/DIB per-pixel route；这不是继续打磨 overlay，只是让文档/测试与当前 baseline 一致。
- 验证已通过：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、
  `cargo test --target x86_64-pc-windows-msvc`、`cargo build --target x86_64-pc-windows-msvc`、
  `cargo test --target x86_64-pc-windows-msvc platform::audio_convert`、Windows ffmpeg ignored runtime smoke、
  Windows retained-audio finish ignored runtime smoke。
- 下一步建议：继续非 overlay 能力闭环，优先做 full Windows recording retained-audio smoke
  (`voice.record_audio=lossless/compact`) 或检查 TUI/history/audio deletion/open path 在 Windows 的真实行为；
  overlay modernization 和 cross-user 仍 deferred。

Phase 10bi Windows overlay baseline closure 已完成：

- 用户确认 Windows overlay 现在“总体上勉强可用/看起来还可以”，但阴影生硬、文字不够原生清晰，
  与 macOS Liquid Glass 质感仍有明显差距。
- 当前 Windows renderer 路线是 Win32 no-activate layered window + Direct2D/DirectWrite + 32bpp DIB +
  `UpdateLayeredWindow`，GDI fallback 保留。该路线适合作为低依赖 daemon-hot-path baseline/fallback，
  不是最终高质感路线。
- 本阶段修正了 DPI/work-area/任务栏避让、Direct2D 96 DPI DIB 坐标、DirectWrite 物理字号、右侧 meta
  对齐、DirectWrite text layout 行数测量和尾部文本截断，降低裁剪、提前空行、滚动不一致问题。
- 决策：停止继续在当前 renderer 上追求最终视觉质感；待 Windows core runtime 其他能力补齐后，单独重写
  Windows modern overlay renderer，优先评估 DirectComposition / Windows Composition / Windows App SDK 等路线。
- Capability 不升级：`overlay.renderer` 仍 partial，`overlay.material`/`overlay.window_anchor` 仍 degraded。
- 验证已通过：`cargo fmt --check`、Windows overlay/layout/direct2d 相关单测、
  `cargo build --target x86_64-pc-windows-msvc`、`cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings`。

Phase 10bh Windows Silero unavailable fallback 已完成：

- 用户接入麦克风后进行 F3 full recording，overlay 显示 `asr interrupted -- nothing pasted`。
- 日志第一处真实错误是 Windows 上 `idle_pause=true` + `voice.vad.backend=silero` 选择了
  `RecordingMode::VadPause`，随后初始化 Silero stub 失败：
  `Silero VAD is not available on this platform until ONNX Runtime provisioning is defined`。
- 本阶段新增 `voice::silero::is_available()`，`RecordingMode::select` 只有在 Silero backend、provider
  `idle_pause=true` 且当前平台已提供 Silero runtime 时才进入 VadPause；Windows/Linux 退回 Continuous
  recording。
- Windows 仍不升级 VAD capability，也不引入 ONNX Runtime provisioning。
- 验证已通过：`cargo fmt --check`、精确 Windows recording mode 单测、
  `cargo test --target x86_64-pc-windows-msvc`、`cargo build --target x86_64-pc-windows-msvc`、
  `cargo clippy --all-targets -- -D warnings`、`cargo test`。
- 下一步需要用户手动重试 F3 full recording，确认不再命中 Silero unavailable，若仍失败再查 ASR/post/clipboard/paste
  的下一处真实错误。

Phase 10bg Windows audio stream runtime smoke 已完成：

- 用户已接入麦克风；Windows `doctor` 现在能看到 `麦克风 (Realtek USB Audio) (48000Hz, 2ch, F32)`，
  `microphone.input.devices: count=1`。
- 本阶段新增 ignored Windows runtime smoke：
  `cargo test --target x86_64-pc-windows-msvc voice::recorder::tests::windows_input_stream_runtime_smoke_receives_pcm_chunks -- --ignored --exact`。
- 该 smoke 在 Windows 本机通过：打开真实默认输入设备，收到 16 kHz mono PCM chunks，然后 discard；默认只证明
  WASAPI/cpal stream start 和 callback delivery。
- 可设置 `SHUOHUA_WINDOWS_AUDIO_REQUIRE_SIGNAL=1` 后运行同一测试，要求录到非静音 peak；这需要用户在测试时对着麦克风发声。
- Capability 仍保持 `partial`，但 reason 从 `diagnostic_probe_only` 收窄为 `input_stream_runtime_smoke`。
- 该阶段仍不验证 ASR、VAD 质量、retained audio conversion、hotkey-triggered full session、clipboard/paste
  或 end-to-end record -> history。

Phase 10bf Windows overlay DPI-aware adaptive width 已完成：

- 用户在接入麦克风后反馈 Windows overlay 右侧被截断，且 4K 显示器大小正常但 1080p 下显得过大。
- Windows renderer 现在把 `overlay.width` 解释为首选逻辑宽度，而不是必须使用的绝对宽度。
- 实际绘制时会根据当前 foreground monitor 的 DPI-aware work area 解析 effective panel width，并将其同时用于
  bottom/center placement、Win32 window size、Direct2D per-pixel surface、GDI fallback rectangles 和 text
  wrapping。
- 这不新增配置项、不改变 theme 字段、不改变 macOS 默认视觉；roomy work area 仍使用用户配置的宽度。
- Capability 不升级：这只是 Windows overlay 布局正确性修复，不覆盖 fullscreen/UAC/multi-monitor 最终 QA。

Phase 10be Windows startup registration error boundary 已完成：

- Windows `shuo service install` / `uninstall` 仍不实现 Task Scheduler、SCM、PowerShell 或 registry
  startup registration。
- 错误文案从泛化 “service management is not implemented” 收窄为 “startup registration is not
  implemented”，并提示当前已实现的 user-session lifecycle 命令：
  `service start/status/restart/stop`。
- 这只是错误边界和诊断清晰度修复，不改变 start/status/restart/stop 行为，不升级 capability。

Phase 10bd Windows path capability diagnostics 已完成：

- Windows `path.open_reveal` capability reason 从 `runtime_not_verified` 收窄为
  `basic_manual_smoke_only`。
- 这只同步用户已确认的 Explorer open/reveal 基础目视结果；不改变 `explorer.exe` backend，不升级
  status。
- UNC、missing path、焦点行为和非交互/多用户会话仍未验证；`explorer.exe` exit code 仍不能作为唯一判断。

Phase 10bc Windows IPC/lifecycle capability diagnostics 已完成：

- Windows `ipc.transport` 和 `daemon.single_instance` capability reason 从旧的
  `runtime_not_verified` 收窄为 `same_user_elevation_smoke_only`。
- Windows `process.probe` capability reason 从 `runtime_not_verified` 收窄为
  `service_lifecycle_smoke_only`。
- 这只同步既有 Windows same-user/elevation/busy/service lifecycle smoke 结论到 doctor/TUI 静态诊断；
  不改 IPC/service/lifecycle 行为，不升级 status。
- cross-user 第二账号/VM 隔离和 longer soak 仍是 deferred manual gate；完成前不得标
  `available`。

Phase 10bb Windows overlay capability diagnostics 已完成：

- Windows overlay capability snapshot 不再用泛化 `win32_overlay_minimal` 描述所有维度。
- doctor/TUI 现在能看到更具体的 backend/reason：
  `win32_direct2d_per_pixel`、`direct2d_per_pixel_runtime_smoke`、`translucent_shadow_no_blur`、
  `win32_topmost_noactivate`、`win32_httransparent`、`win32_foreground_monitor_work_area`、
  `foreground_monitor_screen_anchor_only`。
- 这只是静态诊断文案精确化，不创建 overlay window、不 probe monitor、不升级 capability。
- 用户当前没有音频设备；继续开发时仍优先做不依赖麦克风的 overlay/diagnostics/service/IPC/config 小步。

Phase 10ba Windows shadow tuning 已完成：

- 用户目视反馈 Phase 10ay 阴影“很干净但是有点生硬”。
- Windows Direct2D shadow 改为两组 renderer-owned pass：wide low-alpha ambient shadow + lower/narrower key
  shadow，并用 tapered per-layer alpha 让外缘更柔和。
- `DIRECT2D_SHADOW_OUTSET` 从 10 logical px 增至 14 logical px，给更宽 ambient shadow 留透明 surface 空间。
- 未新增用户配置项，未升级 capability：`overlay.material` 仍是 degraded translucent fallback；这不是 blur/
  Acrylic/Mica/Liquid Glass parity。
- 用户当前没有音频设备；继续开发时优先做不依赖麦克风的 overlay/diagnostics/service/IPC/config 小步。

Phase 10az Windows foreground monitor work area 已完成：

- Windows overlay placement 现在优先用 foreground window 的 nearest monitor work area：
  `GetForegroundWindow` + `MonitorFromWindow` + `GetMonitorInfoW`。
- `SPI_GETWORKAREA` 只作为 monitor lookup 失败时的 fallback。
- 这只是 screen anchoring 的多显示器基础修正；不会跟随 foreground window frame、caret、文本框或
  app-specific geometry。
- Capability 不升级：`overlay.window_anchor` 仍是 degraded `screen_anchor_only`，直到 focused-window
  anchoring 和多显示器目视 QA 完成。

Phase 10ay Windows Direct2D per-pixel shadow polish 已完成：

- Windows Direct2D renderer 现在把 `UpdateLayeredWindow` per-pixel surface 扩大一个 renderer-owned shadow
  outset，并把实际 panel/text/icon 绘制进 inset panel rect。
- 阴影由 Direct2D 在同一 premultiplied-alpha surface 内分层绘制；圆角外透明像素、panel、文字和阴影保持
  同一 renderer 管线控制，不重新启用 DWM backdrop。
- GDI fallback 仍走旧的 tight rounded region，不尝试 shadow。
- 未新增用户配置项，未升级 capability：`overlay.material` 仍是 degraded translucent fallback；这只是
  surface polish，不是 blur/Liquid Glass parity。

Phase 10ax Windows overlay DWM backdrop disabled 已完成：

- 用户目视发现 DWM backdrop probe 后圆角矩形外出现未知背景图像。
- 判断为 DWM rectangular backdrop 与当前 `WS_EX_LAYERED` / `UpdateLayeredWindow` per-pixel rounded surface
  组合不干净；圆角外边界必须优先保证透明。
- 已禁用 `DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_TRANSIENTWINDOW` 路线，Windows overlay 回到 Direct2D
  per-pixel translucent surface。
- 后续若继续做 blur/material，应评估 DirectComposition/Windows Composition 等能同时控制 blur、rounded
  clipping 和文字合成的路线；不要在当前 layered window 上继续堆 DWM backdrop 参数。

Phase 10aw Windows overlay DWM backdrop probe 已完成：

- Windows overlay 曾尝试 best-effort 请求 `DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_TRANSIENTWINDOW`，即
  Windows 11 短生命周期窗口 / Desktop Acrylic-style backdrop 路线，并请求 immersive dark mode。
- 该 probe 后续在 Phase 10ax 被禁用；不要把 Phase 10aw 当作当前启用状态。

Phase 10av Windows overlay state icons 已完成：

- `overlay.text_scale` 允许范围扩大为 `0.8..=2.4`，schema 和 runtime layout clamp 使用同一组常量。
- Windows overlay 的预留 icon 列现在会绘制状态 glyph；GDI fallback 和 Direct2D renderer 都使用
  renderer 自绘形状，不依赖 SF Symbols、SF Pro、Segoe Fluent Icons codepoint 或用户额外安装字体。
- macOS 继续使用 SF Symbols 系统能力；Windows 不追求复刻 SF Symbols 名称，先保证状态语义、可读性和
  无额外字体依赖。
- 本阶段不升级 overlay capability；Windows icon 仍需要用户目视确认大小、对齐和高分屏观感。

Phase 10au Overlay shared scaled layout 已完成：

- `overlay.text_scale` 现在不是单独放大字体；共享 `overlay::layout::overlay_frames(...)` 会同时推导
  overlay 高度、header row frame、body frame、body line height 和字体大小。
- Windows GDI fallback、Windows Direct2D renderer、macOS AppKit renderer 都消费同一套 frame 计算，避免
  `text_scale=1.5` 时文字被旧固定 body/header 矩形挤压。
- Windows 初始 window size、screen placement 高度和 rounded region 也跟随共享高度；底部居中位置仍由
  work area + `panel_frame` 决定。
- macOS 默认 `text_scale=1.0` 时仍保持原始 `BASE_HEIGHT=64.0`，避免默认视觉回退。
- 本阶段只修 layout model 和 renderer 消费，不升级 overlay capability；Windows 4K 视觉仍需要用户目视 QA。

Phase 10at Overlay size preferences 已完成：

- 主配置 `[overlay]` 新增用户偏好 `width` 和 `text_scale`；已有 `max_text_lines` 继续负责最多显示行数。
- 这两个字段不放进 theme：切换 theme 只改变颜色/材质，用户的宽度、文字大小和行数偏好保持稳定。
- `EffectiveOverlayCfg.core` 现在携带 `width` / `text_scale`；Windows 和 macOS renderer 都消费同一语义。
- `overlay::layout` 根据 `width` 和 `text_scale` 推导 body 宽度、行高和 `chars_per_line`，不暴露
  `height` 或 `chars_per_line` 配置，避免和 `max_text_lines` / 实际字体渲染冲突。
- 默认值保持 `width=572.0`、`text_scale=1.0`，因此默认不改变 macOS 现有视觉；Windows 4K 用户可先在
  `%APPDATA%\Shuohua\config.toml` 的 `[overlay]` 下手动调大，例如 `width=680.0`、`text_scale=1.1`。
- 本阶段只增加配置和 renderer 消费，不升级 overlay capability。

Phase 10as Windows per-pixel layered surface 已完成：

- 用户目视确认 Phase 10ar Direct2D 后“稍微好一点，但仍不够清晰”；根因判断为 Direct2D path 仍使用
  `SetLayeredWindowAttributes` 全窗 alpha，导致背景和文字一起半透明。
- Phase 10as follow-up 修复 `UpdateLayeredWindow` 目标点回归：per-pixel surface 更新时 `pptDst` 传
  `NULL`，保留 `SetWindowPos` 已设置的默认底部居中位置，不再把 overlay 移到屏幕左上角。
- Windows Direct2D renderer 改为 top-down 32bpp DIB + `ID2D1DCRenderTarget` /
  `CreateDCRenderTarget` + `BindDC`，再用 `UpdateLayeredWindow` / `AC_SRC_ALPHA` 发布。
- `UpdateLayeredWindow` 使用 `SourceConstantAlpha: 255`：背景像素使用
  `overlay.surface.background_alpha`，文字保持 solid 255-alpha text，避免全窗 alpha 模糊文字。
- Win32 shell 不变：`WS_POPUP`、layered、topmost、tool window、no-activate、`HTTRANSPARENT`
  click-through 仍由 `src/overlay/windows.rs` 管理。
- GDI fallback 保留；只有 fallback 才继续使用 global layered-window alpha。
- Capability 不升级：这只是文字 alpha/合成基础修正，native backdrop、shadow、animation、
  focused-window anchoring、fullscreen/UAC、multi-monitor 和最终视觉 QA 仍未完成。

Phase 10ar Windows Direct2D/DirectWrite renderer foundation 已完成：

- Windows overlay 保留现有 Win32 window shell：`WS_POPUP`、layered、topmost、tool window、
  no-activate、`HTTRANSPARENT` click-through 和 service/daemon lifecycle 不变。
- 新增 Windows-only `src/overlay/windows/direct2d.rs`，用 typed `windows` crate COM bindings 封装
  Direct2D/DirectWrite 初始化、render target resize 和 paint；共享 overlay model/layout、daemon、IPC、
  hotkey、audio、clipboard/paste 不依赖 Direct2D/DirectWrite 类型。
- 第一版使用 `ID2D1HwndRenderTarget` + DirectWrite `IDWriteTextFormat`，先解决 GDI 文本清晰度上限；
  暂不引入 DirectComposition、D3D/DXGI device chain、`UpdateLayeredWindow` per-pixel surface、Acrylic/Mica、
  shadow 或 animation。
- Direct2D/DirectWrite 初始化或 paint 失败时保留 GDI fallback，避免 graphics stack 问题导致 overlay
  完全不可用。
- Capability 不升级：需要用户目视确认文本清晰度，并继续验证 foreground app、fullscreen/UAC、
  multi-monitor 和 material/shadow 决策。

Phase 10aq Windows overlay rounded GDI baseline 已完成：

- Windows overlay 现在把共享 `overlay.surface.corner_radius` 应用到 Win32 window region：
  `CreateRoundRectRgn` + `SetWindowRgn`。
- layered-window opacity 改用共享 `overlay.surface.background_alpha`，不再使用 Windows backend 私有固定
  alpha。
- `CreateFontW` 请求 `CLEARTYPE_QUALITY`，仍使用系统 UI 字体 `Segoe UI` 和 GDI `DrawTextW`。
- Capability 不升级：这只是 rounded/ClearType GDI baseline，不是 DirectWrite/Direct2D，不包含 shadow、
  Acrylic/Mica、动画、focused-window anchoring、fullscreen/UAC 或 multi-monitor 最终视觉 QA。
- 如果用户目视仍觉得 Windows 字体明显比系统 UI 糊，下一步应进入 DirectWrite/Direct2D renderer
  foundation，而不是继续微调 GDI。

Phase 10ap Windows overlay DPI and font baseline 已完成：

- Windows overlay 现在用 `GetDpiForWindow` 计算当前 window DPI scale，并把共享 logical layout 的窗口尺寸、
  位置、文本 rect 和 GDI font size 转成 physical pixels。
- overlay placement 使用 Windows work area (`SPI_GETWORKAREA`) 而不是 raw primary-screen bounds，单屏常见
  场景会避开 taskbar；secondary monitor 的 per-monitor work area 仍是后续 gate。
- Windows prose text 使用系统 UI 字体路径 `Segoe UI`，作为 GDI baseline；macOS 现有 AppKit renderer 使用
  `NSFont::systemFontOfSize` / `boldSystemFontOfSize`，不硬依赖 JetBrains Mono 或 bundled SF Pro。
- 不 bundle SF Pro。若后续需要 monospace/branded fallback，应选择可再分发字体并作为 optional fallback，
  不是 daemon/runtime 硬依赖。
- Capability 不升级：`overlay.renderer` 仍是 `partial`，`overlay.material` / `overlay.window_anchor` 仍是
  `degraded`，因为 DirectWrite/Direct2D、material/shadow/rounding、fullscreen/UAC、multi-monitor 和最终
  视觉 QA 尚未完成。

Phase 10ao Windows minimal overlay backend 已完成：

- Windows overlay 不再是 no-op skeleton；现在创建一个原生 Win32 `WS_POPUP` overlay window，使用
  `WS_EX_LAYERED` / `WS_EX_TOPMOST` / `WS_EX_TOOLWINDOW` / `WS_EX_NOACTIVATE`。
- 第一版只使用 Win32/GDI：translucent layered-window background、basic text drawing、show/hide、
  `OverlayCmd::Quit`，不引入 Tauri/WebView、Direct2D、Skia 或 wgpu。
- backend 复用共享 `OverlayModel` 和 layout/text helpers；Windows 代码只拥有 message pump、window
  creation、visibility、hit testing 和 drawing。
- 新增显式 ignored runtime smoke：
  `cargo test --target x86_64-pc-windows-msvc overlay::windows::tests::runtime_smoke_creates_shows_hides_and_quits_window -- --ignored --exact`。
  本机 smoke 通过：短暂创建/显示/隐藏 Win32 overlay window 并正常退出。
- `doctor` capability summary 现在包含
  `overlay.renderer=partial backend=win32_overlay_minimal reason=runtime_smoke_only`、
  `overlay.always_on_top=partial backend=win32_overlay_minimal reason=runtime_smoke_only`、
  `overlay.input_passthrough=partial backend=win32_overlay_minimal reason=runtime_smoke_only`、
  `overlay.material=degraded backend=win32_overlay_minimal reason=translucent_fallback_only`、
  `overlay.window_anchor=degraded backend=win32_overlay_minimal reason=screen_anchor_only`。
- 本阶段没有验证最终视觉质量、真实 foreground App、mouse/touch/pen passthrough、fullscreen/UAC/
  multi-monitor 或 hotkey/audio/full record -> paste flow；capability 不能升级为 available。
- 用户随后完成手动 overlay smoke：临时把 hotkey 改为 `f16`，启动 daemon 后用合成 F16 触发；
  目视确认 overlay 可见、位置大致正确、不抢焦点、可自动消失/随 daemon stop 消失，点击穿透符合预期。
  这仍不覆盖 fullscreen/UAC/multi-monitor/touch/pen 或最终视觉质量。

Phase 10an Windows hotkey hook backend 已完成：

- Windows hotkey provider 现在使用 `WH_KEYBOARD_LL` low-level keyboard hook，运行在
  `hotkey-wh-keyboard-ll` 专用 OS 线程。
- hook callback 将 Windows virtual key 映射到共享 `Key` 模型，写入既有 4-byte `RawEvent`
  pipe wire format，然后复用共享 `Suppressor` 判断是否 drop foreground event。
- modifier key transition 在 Windows 下转成 `FlagsChanged`，并携带 post-transition `ModMask` snapshot，
  保持与现有 tracker contract 对齐。
- 新增显式 ignored runtime smoke：
  `cargo test --target x86_64-pc-windows-msvc hotkey::provider_windows::tests::hook_runtime_smoke_receives_synthetic_f16_down_up -- --ignored --exact`。
  本机 smoke 通过：hook 安装后用 `SendInput` 合成 F16 down/up，pipe 读回对应事件。
- Windows `desktop.hotkey` 和 `desktop.hotkey_suppression` capability 现在是
  `partial/wh_keyboard_ll/runtime_smoke_only`；不能升级为 available，直到真实 foreground app、IME、
  remote desktop、UAC/elevation 和 hold-to-record 行为完成验证。
- 本阶段没有启动 audio、overlay、clipboard/paste、provider runtime 或 full record -> paste flow。

Phase 10am Windows paste injection backend 已完成：

- Windows `platform::autotype::paste()` 现在调度到 Win32 `SendInput` backend，发送
  Control down、V down、V up、Control up 的 Ctrl+V 序列。
- 新增显式 ignored runtime smoke：
  `cargo test --target x86_64-pc-windows-msvc platform::windows::autotype::tests::paste_runtime_smoke -- --ignored --exact`。
  本机用临时 WinForms textbox 做前台目标验证：先写入剪贴板
  `shuohua-paste-smoke-20260625-winforms`，再执行 ignored paste smoke，textbox 读回同一内容。
- Windows `desktop.text_injection` capability 现在是
  `partial/sendinput_ctrl_v/runtime_smoke_only`；`desktop.clipboard` 仍是独立的
  `partial/win32_clipboard_unicode/write_only_runtime_smoke`。
- 本阶段没有实现 hotkey、overlay、audio 或 record -> ASR -> post -> paste 全链路；paste capability
  不能升级为 available，直到真实目标 App、UAC/elevation 边界和完整 session 验证完成。

Phase 10al Windows clipboard write backend 已完成：

- Windows `platform::clipboard::write_string` 现在调度到 Win32 backend，使用
  `OpenClipboard` / `EmptyClipboard` / `SetClipboardData(CF_UNICODETEXT)` 写 Unicode 文本。
- backend 使用 movable global memory；`SetClipboardData` 成功后句柄所有权转交系统，失败路径释放内存。
- 新增显式 ignored runtime smoke：
  `cargo test --target x86_64-pc-windows-msvc platform::windows::clipboard::tests::clipboard_write_runtime_smoke -- --ignored --exact`。
  本机用 `SHUOHUA_WINDOWS_CLIPBOARD_SMOKE_TEXT=shuohua-clipboard-smoke-20260625-🙂` 执行并通过，随后
  PowerShell STA `[System.Windows.Forms.Clipboard]::GetText()` 读回同一内容。
- Windows `desktop.clipboard` capability 现在是
  `partial/win32_clipboard_unicode/write_only_runtime_smoke`；`desktop.text_injection` 仍是 unsupported。
- 本阶段没有实现 `SendInput` paste、hotkey、overlay、audio 或 record -> paste 全链路；clipboard
  capability 不能升级为 available，直到目标 App 和 elevation 边界完成验证。

Phase 10ak Windows profile route diagnostics 已完成：

- `shuo doctor` 现在复用当前 foreground app `AppContext`，同时打印
  `desktop.active_app.current` 和只读 `profile.route.current`。
- `profile.route.current` 复用 daemon session start 的
  `AppIdentity::current_from_app_context(&AppContext)` + `ProfileRouteCfg::matching_profiles` 语义，
  可区分 default fallback、单一 route 命中和 duplicate-match 配置错误。
- Windows 本机 runtime smoke 中，Windows Terminal 前台时输出
  `profile.route.current: selected=agent source=route matches=agent`。
- 本阶段未启动录音、provider runtime、hotkey、overlay、clipboard 或 paste；capability 不升级。
- Windows `doctor` 的麦克风、daemon/service 和 permissions next-step 已收窄为 Windows 语义：
  无默认输入设备时提示 Windows Settings > System > Sound；Windows service install/startup registration
  仍明确为未实现；Windows permission probe 仍诚实显示 unavailable。

Phase 10aj Windows active app identity diagnostics 已完成：

- Windows `platform::desktop::frontmost_app()` 现在通过 `GetForegroundWindow` /
  `GetWindowThreadProcessId` / `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` /
  `QueryFullProcessImageNameW` 解析 foreground window owner process。
- `AppContext` 新增 `windows_app_user_model_id` 和 `windows_exe_name`；本阶段只填
  `windows_exe_name`，并用 executable file name 派生 display `app_name`。完整进程路径不进入
  doctor、state、history 或 IPC。
- `AppIdentity::current_from_app_context(&AppContext)` 现在在 Windows 下使用
  `windows_app_user_model_id` / `windows_exe_name`，所以
  `profile.routes.<profile>.windows.exe_name` 已有真实 runtime 输入；`app_user_model_id` 仍是预留
  matcher，AUMID lookup 未实现。
- `shuo doctor` 新增 `desktop.active_app.current` 低风险诊断行，用于 Windows runtime smoke；该诊断
  不启动录音、hotkey、overlay、clipboard 或 paste。
- Windows `desktop.active_app` capability 更新为
  `partial/foreground_window_process_exe/exe_name_only`；不能升级为 `available`，因为 AUMID、更多
  foreground window 类型和真实录音 session profile 命中还未验证。

Phase 10ai Platform-specific profile routes 已完成：

- `config.toml` 的 profile 路由从旧的 `[profile] agent = ["bundle.id"]` 改为
  `[profile.routes.<profile>.<platform>]`，profile 文件、ASR provider config、post chain、prompt 和
  hotwords 仍三端共享。
- 新增 `AppIdentity` / `ProfileRoutes` / platform matcher config model：macOS 用 `bundle_id` 精确匹配；
  Windows 预留 `app_user_model_id` 和 `exe_name`，大小写不敏感；Linux 预留 `desktop_id`、`wm_class`、
  `process_name`。
- 旧的顶层 profile route array 已明确拒绝；当前没有外部用户，未做兼容迁移逻辑。
- daemon 入口改为通过当前平台 `AppIdentity::current_from_app_context(...)` 选择 profile。Windows
  `exe_name` route 在 Phase 10aj 后已有 runtime 输入；Linux active app identity backend 还未实现，
  所以 Linux 仍会诚实落到 `profile.default`。
- starter config 模板已输出 `profile.routes.chat.*` 和 `profile.routes.agent.*`，并保守填入 macOS、
  Windows、Linux matcher 样例。
- 本机 `%APPDATA%\Shuohua\config.toml` 已手动迁移到新 `profile.routes` 结构；没有改 ASR/LLM key
  文件。

Phase 10ah Windows audio capture diagnostics 已完成：

- 新增 `platform::audio_capture` 诊断 facade，集中 cpal default host/default input/input device
  enumeration；`voice::recorder::probe_default_input()` 现在转发到该 facade，录音启动路径的 stream
  行为不变。
- `shuo doctor` 现在打印 `microphone.input: backend=...`，并尽量打印
  `microphone.input.devices: count=...`。即使本机没有 default input device，也会报告 Windows backend
  和 input device count，便于后续手动麦克风验证前定位设备枚举状态。
- Windows `audio.capture` capability 从 generic unsupported 调整为
  `partial/cpal_wasapi/diagnostic_probe_only`；这只表示诊断探针存在，不代表真实录音、权限弹窗、
  sample conversion、silence/noise floor、retained audio 或持续采集已验证。
- 本机 Windows `doctor` 结果：`microphone.input: backend=cpal_wasapi ERROR no default input device`，
  `microphone.input.devices: count=0`，并且 capability summary 包含
  `audio.capture=partial backend=cpal_wasapi reason=diagnostic_probe_only`。

Phase 10ag Windows service lifecycle smoke helper 已完成：

- `scripts/windows-ipc-smoke.ps1` 不再直接用 `Start-Process --daemon` 启动 daemon；现在通过
  `shuo service start` 覆盖真实 CLI lifecycle 入口。
- helper 现在验证重复 `service start` 的 idempotency（PID 不变）、`service restart` 后 daemon 仍
  running 且 PID 变化、第二个 `--daemon` 仍失败、20 并发 `service status` busy smoke、`service stop`
  后无残留 daemon。
- PowerShell native command redirection / `Start-Process -Wait` 在 daemon 子进程存在时会等待过久；
  helper 改为 `Start-Process -PassThru` + `.WaitForExit(timeout)`，避免把 daemon 子进程树当作 client
  命令的一部分等待。
- 本机 elevated helper 输出通过：`service_start_exit=0`、`service_start_again_exit=0`、
  `service_restart_exit=0`、`after_start_again_pid == daemon_pid`、`after_restart_pid != daemon_pid`、
  `busy_exit_0=20`、`service_stop_exit=0`、`failures=[]`。
- 追加 same-user elevated soak：`.\scripts\windows-ipc-smoke.ps1 -StopExisting -ClientCount 100`
  通过，`busy_exit_0=100`、`busy_exit_files=100`、`busy_nonzero=[]`、`service_stop_exit=0`、
  `failures=[]`。

Phase 10af Windows raw Named Pipe client access mask 已完成：

- Windows IPC client connect 不再使用 Tokio `ClientOptions::new().open(...)` 的
  `GENERIC_READ` / `GENERIC_WRITE` 路径。
- 新路径使用 raw `CreateFileW` 打开 scoped Named Pipe，desired access 为
  `FILE_READ_DATA | FILE_WRITE_DATA`，保留 `FILE_FLAG_OVERLAPPED` 和
  `SECURITY_IDENTIFICATION | SECURITY_SQOS_PRESENT`，随后用 `NamedPipeClient::from_raw_handle` 接回
  Tokio I/O。
- 既有 `ERROR_PIPE_BUSY` bounded retry policy 保持不变；access/scope/security 错误仍保持可见。
- Windows same-user elevated runtime smoke 通过：`service start/status/restart/stop` 正常，第二个
  `--daemon` 仍失败，stop 后无残留同路径 `shuo` 进程。
- `scripts/windows-ipc-smoke.ps1 -StopExisting` 通过：20/20 busy clients exit 0，
  `service_stop_exit=0`，`after_stop_status_exit=0`，`failures=[]`。
- Windows `ipc.transport` capability 仍为 `partial/runtime_not_verified`；cross-user 第二账号/VM
  隔离和 longer runtime soak 仍是 deferred manual gate。

Phase 10ae Windows user-session service start/restart 已完成：

- Windows `shuo service start` 现在会先查询当前 scoped Named Pipe `DaemonStatus`；daemon 已运行时只打印
  status 和 `already running windows.user`，不会生成第二个 daemon。
- daemon absent 时，`service start` 只 spawn 当前 executable 的 `--daemon` 子进程，重定向 stdout/stderr
  到 `%LOCALAPPDATA%\Shuohua\service.start.*.log`，并有界等待当前用户/登录会话 Named Pipe ready。
- Windows `shuo service restart` 先查询 daemon；运行中则复用既有 IPC `Shutdown` + PID exit wait，
  然后再执行 `start`。daemon absent 时，`restart` 退化为 explicit start。
- `install` / `uninstall` 仍 unsupported；没有调用 Task Scheduler、SCM、PowerShell 或 registry APIs。
  `service status` dry-run 行现在明确 `start=explicit_process startup_registration=unsupported`。
- Windows `service.manager` capability 仍为 `partial`，backend `windows_user_session`，reason
  `user_session_start_stop_only`；这不代表 startup registration 或 cross-user runtime 已完成。
- 本机 elevated runtime smoke 通过：`service start` 启动 daemon，重复 `service start` 不创建第二个
  daemon，`service restart` 更换 PID，`service stop` 后无残留同路径 `shuo` 进程。

Phase 10ad Windows smart fallback Named Pipe probe 已完成：

- Windows `run_smart_fallback()` 不再把 endpoint 永远视为 absent；现在用现有 Named Pipe transport
  probe 当前 scoped endpoint，能区分 pipe-not-found、pipe-busy/present 和 access/scope 类错误。
- Absent endpoint 仍只启动当前 executable 的 `--daemon` 子进程并等待 Named Pipe ready；没有调用
  Task Scheduler、SCM、PowerShell 或 registry APIs，也没有实现 service install/start。
- 新增 Windows fallback 单测覆盖 pipe-not-found、pipe-busy 和 live Named Pipe probe。
- 本机 elevated runtime smoke 通过：无参数 `shuo.exe` 可拉起 daemon，随后 `service status` 显示
  `daemon: running ... state=Idle`，`service stop` exit 0，最后无残留 `shuo` 进程。

Phase 10ac Windows service stop IPC shutdown 已完成：

- Windows `shuo service stop` 现在只通过当前用户/登录会话的 Named Pipe 发送既有
  `Command::Shutdown`，收到 `DaemonStatus` 后有界等待 daemon PID 退出。
- `install` / `uninstall` / `start` / `restart` 仍返回 unsupported；没有调用 Task Scheduler、SCM、
  PowerShell 或 registry APIs，也没有实现 daemon auto-start。
- Runtime smoke 首次暴露 `OpenProcess` 句柄成功不能代表进程仍 active；Windows
  `platform::lifecycle::process_exists` 已改为 `OpenProcess` 后调用 `GetExitCodeProcess`，只把
  `STILL_ACTIVE` 视为 running。
- `scripts/windows-ipc-smoke.ps1` 现在覆盖 `service stop` 和 after-stop status；elevated same-user
  smoke 通过，`service_stop_exit=0`、`daemon_running_after_stop=false`、`failures=[]`。

Phase 10ab Windows service status IPC error classification 已完成：

- Windows `service status` 不再把所有 `connect_default()` 失败都当成 `daemon: not running`。
- 现在只有 `ERROR_FILE_NOT_FOUND` / `ERROR_PATH_NOT_FOUND` 被归类为 daemon absent；access denied、
  scope/security 错误、busy 超时和其他 IPC 错误会保留为错误返回，避免 cross-user/access-mask 问题被
  dry-run status 吞掉。
- 新增 Windows service 单测固定 absent-vs-access-denied 分类；runtime helper smoke 仍通过。

Phase 10aa Windows abandoned mutex behavior 已完成：

- Windows `platform::lifecycle` 不再把 `WAIT_ABANDONED` 静默当作普通 acquire；现在映射为
  `AbandonedRecovered` 并记录 warning，然后继续持有 guard。这符合 Win32 mutex 被 abandoned 后
  当前线程获得所有权的语义。
- 新增纯函数单测固定 `WAIT_OBJECT_0` / `WAIT_ABANDONED` / `WAIT_TIMEOUT` 映射。
- docs/cross-platform/windows.md 和 platform-capabilities.md 已同步：真实 crash/abandon smoke 仍待
  Windows runtime 验证，但行为边界已不再未定义。

Phase 10z Windows capability next-step sync 已完成：

- Windows `ipc.transport` / `daemon.single_instance` capability 仍保持 `partial/runtime_not_verified`，
  但 summary/next_step 不再停留在纯 compile backend 说法，已同步 same-user/elevation smoke 通过后的
  真实剩余项：cross-user 隔离、client access-mask narrowing、abandoned mutex 等。
- Windows `path.open_reveal` capability 仍保持 `partial/runtime_not_verified`，但 summary 记录基础
  Explorer open/reveal 已人工确认，next_step 指向 UNC、missing path、non-interactive session 等更广
  的路径/会话验证。
- `docs/cross-platform/platform-capabilities.md` 和 `tests/platform_layout.rs` 已同步，避免 doctor/TUI
  静态诊断继续提示已经完成的 same-user Named Pipe runtime smoke。

Phase 10y Windows IPC same-user smoke helper 已完成：

- 用户确认 cross-user 第二账号/VM 验证可以后移；它现在是 deferred manual gate，不阻塞继续做
  same-user Windows IPC/lifecycle hardening，但 Windows IPC / daemon single-instance capability 仍不得升级
  为 `available`。
- 新增 `scripts/windows-ipc-smoke.ps1`，用于重复跑 same-user IPC smoke：artifact identity、
  daemon start、`service status`、second daemon rejection、20 并发 busy smoke、after-busy status，
  并输出 `summary.json`。
- `docs/cross-platform/windows-runtime-validation.md` 已指向该 helper，并在 Cross-User Smoke 中明确
  deferred manual gate 语义。
- 该 helper 只覆盖当前 Windows 用户/当前 integrity shell；elevated/non-elevated 交叉仍需要分别从
  normal/admin shell 调用或手动跑矩阵，cross-user 仍需要第二 Windows 用户或 VM。

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
  - `shuo.exe doctor` 能运行并使用 `%APPDATA%\Shuohua`；Phase 10ai 后旧 profile route schema 错误
    已消失。当前返回 1 的 blocking issue 是无默认输入设备和 Windows permission probe 未实现；另有
    `post/llm/anthropic.toml`、`post/llm/openai.toml` draft key empty warning。
  - 本机 `%APPDATA%\Shuohua\config.toml` 已迁移到 `[profile.routes.<profile>.<platform>]`；后续填
    `asr/doubao.toml` 和 `post/llm/deepseek.toml` 的 key 不需要再改 route schema。
  - Phase 10ah doctor audio diagnostics 通过执行：本机无默认输入设备时仍打印
    `microphone.input: backend=cpal_wasapi ERROR no default input device` 和
    `microphone.input.devices: count=0`。
  - Windows capability summary 现在包含
    `audio.capture=partial backend=cpal_wasapi reason=diagnostic_probe_only`；这仍不是录音可用声明。
  - Phase 10aj doctor active app diagnostics 通过执行：本机 Windows Terminal 前台时打印
    `desktop.active_app.current: exe_name=WindowsTerminal.exe app_name=WindowsTerminal`；capability summary
    包含
    `desktop.active_app=partial backend=foreground_window_process_exe reason=exe_name_only`。
  - Phase 10ak doctor profile route diagnostics 通过执行：本机 Windows Terminal 前台时打印
    `profile.route.current: selected=agent source=route matches=agent`。`doctor` 汇总的麦克风、daemon/
    service、permissions next-step 已使用 Windows 语义；当前 `doctor` exit 1 仍只因无默认输入设备。
  - Phase 10al clipboard write runtime smoke 通过：ignored test 写入
    `shuohua-clipboard-smoke-20260625-🙂`，PowerShell STA clipboard readback 返回同一内容。
    `doctor` capability summary 现在包含
    `desktop.clipboard=partial backend=win32_clipboard_unicode reason=write_only_runtime_smoke`。
  - Phase 10am paste injection runtime smoke 通过：临时 WinForms textbox 前台目标读回
    `shuohua-paste-smoke-20260625-winforms`；Notepad smoke 尝试未作为结论，因为 Windows 11 Notepad
    未暴露可用 `MainWindowHandle`。`doctor` capability summary 现在包含
    `desktop.text_injection=partial backend=sendinput_ctrl_v reason=runtime_smoke_only`。
  - Phase 10an hotkey hook runtime smoke 通过：ignored test 安装 `WH_KEYBOARD_LL` hook 后用
    `SendInput` 合成 F16 down/up，pipe 读回对应 `RawEvent`。`doctor` capability summary 现在包含
    `desktop.hotkey=partial backend=wh_keyboard_ll reason=runtime_smoke_only` 和
    `desktop.hotkey_suppression=partial backend=wh_keyboard_ll reason=runtime_smoke_only`。
  - Phase 10an 后 `service start; service status; service stop` 单步 smoke 通过，daemon 可启动到
    `state=Idle` 并正常停止；该 smoke 未触发录音，因为本机仍无默认麦克风。
  - Phase 10ao overlay runtime smoke 通过：ignored test 短暂创建/显示/隐藏 Win32 overlay window 并
    正常退出。`doctor` capability summary 现在包含
    `overlay.renderer=partial backend=win32_overlay_minimal reason=runtime_smoke_only`、
    `overlay.material=degraded backend=win32_overlay_minimal reason=translucent_fallback_only`、
    `overlay.input_passthrough=partial backend=win32_overlay_minimal reason=runtime_smoke_only` 和
    `overlay.window_anchor=degraded backend=win32_overlay_minimal reason=screen_anchor_only`。
  - Phase 10ao 后 `service start; service status; service stop` 单步 smoke 通过，确认新 Win32 overlay
    backend 不阻塞 daemon 启停；该 smoke 未进入真实录音。
  - 用户手动 overlay smoke 通过：临时 `trigger = "f16"` 后启动 daemon，用合成 F16 触发无麦克风错误路径；
    目视确认 overlay 可见、位置大致正确、不抢焦点、可消失，点击穿透符合预期。该结论仍不覆盖
    fullscreen/UAC/multi-monitor/touch/pen 或最终视觉质量。
  - Phase 10ap overlay DPI/font baseline 通过 Windows native 验证：Windows target tests/build 通过，
    ignored overlay runtime smoke 通过，`service start; service status; service stop` 单步 smoke 通过。
    当前 `doctor` exit 1 仍是无默认输入设备；overlay capability 仍保持 partial/degraded。
  - Phase 10ap 字体决策：三端优先使用系统 UI 字体；macOS 不 bundle SF Pro，Windows 不要求
    JetBrains Mono。后续若需要额外 fallback，只选择可再分发字体并作为 optional packaged fallback。
  - Phase 10aq overlay rounded GDI baseline 通过 Windows native 验证：`platform_layout windows_overlay`
    guard、Windows overlay unit tests、ignored overlay runtime smoke、Windows target tests/build 均通过。
    `service start; service status; service stop` 单步 smoke 通过，确认 rounded region / ClearType GDI
    改动不阻塞 daemon lifecycle。该结论仍需要用户目视确认真实 overlay 质感；capability 不升级。
  - Phase 10ar Direct2D/DirectWrite renderer foundation 通过 Windows native 验证：`platform_layout
    windows_overlay` guard、Windows overlay unit tests、ignored overlay runtime smoke、Windows target
    tests/build 均通过。`service start; service status; service stop` 单步 smoke 通过，确认 Direct2D
    renderer foundation 不阻塞 daemon lifecycle。该结论仍需要用户目视确认文本清晰度；capability
    不升级。
  - 无参数 `shuo.exe` smart fallback 在 daemon absent 时可启动当前 executable 的 `--daemon` 子进程，
    并等到 scoped Named Pipe ready。
  - `shuo.exe service status` 在 daemon running/not running 两种状态下均通过，且只做 dry-run/status。
  - Phase 10ai 后单步 runtime smoke 通过：`service start` 启动 daemon，单独 `service status` 返回
    `daemon: running ... state=Idle`，`service stop` exit 0 且无残留同路径 `shuo` 进程。不要用
    PowerShell native redirection/复合命令等待 `service start` 拉起的 daemon 子进程树；必要时用现有
    `scripts/windows-ipc-smoke.ps1` 或单步命令。
  - `shuo.exe service start` 可在 daemon absent 时启动当前 executable 的 `--daemon` 子进程；daemon
    已运行时重复 start 只报告 already-running，不创建第二个 daemon。
  - `shuo.exe service restart` 可先 IPC stop 当前 daemon，再启动新 daemon；本机 smoke 中 restart 前后
    PID 发生变化。
  - `shuo.exe service stop` 通过 IPC shutdown 停止运行中 daemon；after-stop `service status` 显示
    `daemon: not running`。
  - `shuo.exe --daemon` + scoped Named Pipe `DaemonStatus` 通过。
  - 第二个 `shuo.exe --daemon` 明确失败，单实例 guard 生效。
  - same-user medium/elevated 和 elevated/medium 交叉矩阵通过，修复后不再拆成两个 daemon runtime。
  - Windows IPC client 已切到 raw `CreateFileW` explicit access mask；
    `ipc::transport::imp::tests::client_open_uses_narrow_explicit_access_mask` 通过。
  - `scripts/windows-ipc-smoke.ps1 -StopExisting` 在 elevated session 通过，并覆盖 service
    start/idempotent start/restart/status/busy/stop：20/20 busy clients exit 0，
    `service_restart_exit=0`，`service_stop_exit=0`，`after_stop_status_exit=0`，helper 输出
    `failures: []`。
  - `scripts/windows-ipc-smoke.ps1 -StopExisting -ClientCount 100` 通过，100/100 busy clients exit 0，
    helper 输出 `failures: []`。
  - Explorer direct open/reveal 工具会话仍返回 1，但用户目视确认窗口打开/reveal 生效；不要只看
    `explorer.exe` exit code。
- 未在本 Windows session 跑 `make check-windows` / `make check-linux-cross`；本阶段验证重点是
  Windows native build/test/runtime smoke。

下一步：

- 若要让 `shuo.exe doctor --runtime` 继续往 ASR/LLM provider runtime 走，先确认当前 profile 实际引用的
  provider/component key 已填写；未被 profile 引用的 draft `anthropic/openai` 空 key 只是 warning。
- Windows active app identity backend 当前只覆盖 foreground process `exe_name`；后续若需要 packaged app
  或 Store app 精准匹配，要单独实现 AUMID lookup。找不到 identity 时仍应落回 `profile.default`。
- Windows audio capture smoke 仍需要用户接入/选择默认麦克风，并在 Windows Privacy & Security 中确认
  终端/可执行文件麦克风权限；没有默认 input device 时不要进入真实录音验收。
- 如果暂时不做麦克风、AUMID 或真实录音 session profile 命中测试，可继续做不依赖手动桌面交互的
  诊断/guard 小步，但不要升级 `audio.capture`、`desktop.active_app`、IPC 或 daemon capability。
- cross-user 第二账号/VM 隔离已后移为 deferred manual gate，没有第二用户前不要升级 Windows
  IPC/daemon capability。
- overlay、hotkey、clipboard/paste 都必须在 Windows runtime 上手动验证后才允许 capability 升级。
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
下一步是 Phase 10z Windows IPC/lifecycle hardening；cross-user 第二账号/VM 隔离验证已后移为
deferred manual gate。
Phase 7b/8b overlay backend skeleton、Phase 3b IPC transport cfg boundary、Phase 10a
cross-check baseline、Phase 10b TUI capability diagnostics、Phase 10c Docker/cross Linux
check baseline、Phase 10d Linux compile-time capability sync、Phase 3c Windows Named Pipe
transport compile backend、Windows IPC capability sync、Phase 10i audio convert facade、
Phase 10j Windows lifecycle primitive compile backend、Phase 10k Windows service dry-run/status
skeleton、Phase 10l non-macOS desktop capability truthfulness、Phase 10w elevation split 修复和
Phase 10y Windows IPC same-user smoke helper 已完成。先查看最新
diff/commit 和验证结果。
保持 macOS 不回退，不引入 GUI/WebView。不要把 Windows Named Pipe compile backend 当成实机
runtime 验收。不要添加 GitHub Actions Windows artifact job；Windows 机器作为本地开发/build/runtime
测试环境，通过 GitHub 同步代码。cross-user、audio、overlay、hotkey、clipboard 验证需要用户目标系统。
```
