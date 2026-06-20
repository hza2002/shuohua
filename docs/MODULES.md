# 模块规划

`src/` 只包含已实现的模块。路径细节看 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)。

## 已实现

```
src/
├── main.rs                              # clap 入口；--daemon / 子命令 / smart fallback 分发
├── daemon/
│   ├── mod.rs                           # daemon 对 main 的公开入口 re-export
│   ├── fallback.rs                      # shuo 无 daemon 时智能启动 --daemon，再进入 TUI
│   ├── lock.rs                          # daemon single-instance flock
│   ├── process.rs                       # daemon 进程 bootstrap：log/config/i18n/overlay/tokio thread
│   ├── runtime.rs                       # tokio daemon 主循环：IPC/reload/hotkey/session lifecycle 编排
│   ├── hotkey_input.rs                  # RawEvent pipe bridge + Suppressor binding/cancel-active 更新
│   ├── active_session.rs                # active recording task 的 Stop/Cancel/finished 小封装
│   └── session_start.rs                 # profile/post/asr → SessionParams；startup error → i18n overlay error
├── cli/
│   ├── mod.rs                           # clap 子命令分发
│   ├── doctor.rs                        # shuo doctor：本地配置诊断；--runtime 显式跑 ASR/LLM 可运行性检查入口
│   ├── config_template.rs               # shuo config-template：导出 registry 模板
│   └── service.rs                       # launchd install/uninstall/start/stop/restart/status
├── config/
│   ├── mod.rs                           # config module root；top-level config API re-export + submodules
│   ├── main.rs                          # ~/.config/shuohua/config.toml schema/parse/path helpers
│   ├── paths.rs                         # XDG config path helpers shared by config loaders
│   ├── spec.rs                          # shared field/spec metadata + validation diagnostics
│   ├── schema.rs                        # shared config schema registry + description i18n keys
│   ├── inventory.rs                     # structured Configure/doctor inventory scan
│   ├── diagnostics/                     # full-tree local config diagnostics shared by doctor/Configure
│   │   ├── mod.rs                       # diagnostics facade and tests
│   │   ├── report.rs                    # diagnostic report types and helpers
│   │   ├── runtime_plan.rs              # ASR/LLM runtime check target planning
│   │   └── scan.rs                      # filesystem scan, TOML validation, reference checks
│   ├── template/                        # official config template registry + build-generated theme registry + LLM component creation
│   │   ├── mod.rs                       # template facade and tests
│   │   ├── registry.rs                  # static templates and embedded theme presets
│   │   ├── render.rs                    # schema/comment-driven TOML rendering
│   │   └── llm_wizard.rs                # LLM component draft/render/create helpers
│   ├── profile.rs                       # profile/*.toml schema + route loading
│   ├── post/                            # post component config namespace
│   ├── asr/                             # ASR provider config loaders
│   └── theme.rs                         # theme TOML parse/merge + builtin fallback + effective TUI/overlay theme
├── log.rs                                # tracing 初始化：daily file appender、本地时间格式、TTY mirror
├── reload.rs                             # notify watcher + watch::Sender 广播；overlay/i18n/hotkey subscriber；UDS 手动 reload 复用同一路径
├── platform/
│   ├── mod.rs                           # shared OS capability namespace
│   ├── daemon.rs                        # DaemonPlatform adapter：frontmost app + hotkey event tap 平台边界
│   └── macos/
│       ├── mod.rs                       # macOS shared adapters
│       ├── clipboard.rs                 # NSPasteboard 写文本
│       ├── autotype.rs                  # CGEventPost Cmd+V
│       ├── window.rs                    # CGWindow + AX 拿 focused window 几何，给 overlay 定位
│       └── app_context.rs               # frontmost app bundle id / 名字
├── hotkey/
│   ├── mod.rs                           # 4 字节 RawEvent 线协议 + 公共类型 re-export
│   ├── bindings.rs                      # trigger/cancel binding 集合 + cancel-first TrackerSet
│   ├── combo.rs                         # Combo / ModMatcher / ModMask / Side / ModType + 精确匹配函数
│   ├── key.rs                           # macOS keycode → Key 解码
│   ├── parse.rs                         # 完整 grammar：modifier+key / modifier-only / :double（DESIGN §2.4）
│   ├── tracker.rs                       # 纯函数状态机：纯键 / combo / modifier-only + 双击窗口（500ms hold, 400ms double）
│   ├── suppressor.rs                    # 纯函数 suppress：按 trigger 类型分发，§5 不变量 8 down/up 配对吞
│   ├── proptests.rs (#[cfg(test)])      # Tracker + Suppressor 跟参考模型逐步等价的 proptest
│   └── provider_darwin.rs               # CGEventTap (Default 模式) → CFRunLoop → pipe + 真吞事件 + CGEventFlags → ModMask 解码
├── asr/
│   ├── mod.rs
│   ├── types.rs                         # AsrProvider/Session trait + AsrEvent + AsrError；Partial 尾巴语义
│   ├── fake.rs    (#[cfg(test)])        # 测试用 FakeProvider
│   └── providers/
│       ├── mod.rs
│       ├── apple.rs                     # macOS 26 SpeechAnalyzer provider；canonical PCM → Swift helper → Partial/Segment
│       ├── apple_helper.swift           # Swift-only SpeechAnalyzer bridge；build.rs 编译后嵌入 shuo
│       └── doubao.rs                    # bigmodel_async WS + 二进制帧 + Partial/Segment 映射 + DriftProbe (debug-only)
├── post/
│   ├── mod.rs                           # PostProcessor trait + PipelineText + run_chain
│   ├── app_context.rs                   # post 层 AppContext 入口；macOS 复用 platform::macos
│   ├── zh_filter.rs                     # ZhFilter
│   └── llm.rs                           # LlmCleanup；OpenAI-compatible / Anthropic native 一次性调用
├── voice/
│   ├── mod.rs
│   ├── recorder.rs                      # cpal 流式：F32 → 16k mono s16le → mpsc + 临时 WAV writer
│   ├── audio.rs                         # retained audio：临时 WAV → FLAC/AAC，路径与失败清理
│   ├── finish.rs                        # 公开录音入口 + post/dispatch/history/UI completion
│   ├── engine.rs                        # Continuous / VadPause Active/Idle 引擎 + session 切换
│   ├── capture.rs                       # SegmentCapture / SessionCapture 数据模型 + samples_to_ms / instant_to_datetime
│   ├── finalize.rs                      # provider session 收口：is_last → Final/Segment/Done/timeout
│   ├── history_build.rs                 # HistoryRecord 构造 / append + PipelineStep → PipelineStepHistory
│   ├── post_dispatch.rs                 # post chain 执行 + dispatch::dispatch → DispatchOutcome
│   ├── meter.rs                         # 从已有 PCM/VAD 流聚合 50ms audio meter，供 UDS/TUI 画 waveform
│   ├── observer.rs                      # dev observer：VAD shadow trace sidecar（feature=dev；默认 ZST no-op）
│   ├── vad.rs                           # VAD frame/state 边界 + speech/silence hysteresis controller
│   ├── silero.rs                        # Silero VAD 帧检测器（默认 build）
│   ├── timeline.rs                      # Sample-indexed PCM ring buffer + slice_from（resume 用）
│   └── dispatch.rs                      # 剪贴板 + 可选 Cmd+V
├── state/
│   ├── mod.rs                           # StateStore + 原子 subscribe_with_snapshot + StateEvent broadcast
│   └── history.rs                       # monthly history JSONL append-only writer（schema 见 SCHEMA.md §2）
├── ipc/
│   ├── mod.rs                           # IPC 子模块
│   ├── protocol.rs                      # line-delimited JSON over UDS；Command/Event serde schema
│   ├── server.rs                        # UnixListener；subscribe snapshot；per-client fanout；history 查询
│   └── client.rs                        # TUI/smart fallback 共用 UnixStream framing helper
├── tui/
│   ├── mod.rs                           # ratatui 主循环；Status/History/Configure 三页
│   ├── page.rs                          # 三页共享的 Page trait + key outcome
│   ├── status.rs                        # Status 状态、事件归并与渲染
│   ├── history.rs                       # History 查询、音频 open/reveal/delete 与渲染
│   ├── configure.rs                     # Configure 状态、wizard、doctor 与渲染
│   ├── config_actions.rs                # Configure editor/Finder launcher helpers
│   ├── panes.rs                         # 顶层 tabs/page/footer 布局与页面分发
│   ├── keybindings.rs                   # Tab/Shift-Tab + 1/2/3 翻页；vim/方向键滚动
│   └── settings.rs                      # Configure inventory rows；脱敏展示 secret 字段
├── overlay/
│   ├── mod.rs                           # cfg(target_os) 路由 + pub use
│   ├── command.rs                       # OverlayCmd + OverlayState + TextKind + OverlayHandle
│   ├── model.rs                         # OverlayModel + Notice + apply + tick(now) + TickOutcome
│   ├── layout.rs                        # 平台无关布局：LayoutFrame + 纯文本/几何函数
│   └── macos/
│       ├── mod.rs                       # pub fn run(rx, cfg)
│       ├── view.rs                      # NSPanel 主循环 + control 更新 + 动画 + NSTimer 驱动 tick
│       ├── chrome.rs                    # NSGlassEffectView + SkyLight SPI + HUD fallback + 背景层
│       └── debug.rs   (#[cfg(debug_assertions)])  # NSGlassEffectView SPI 探针
└── i18n/
    ├── mod.rs                            # 内部入口：init/resolve_lang/tr/tr_lang/Lang + t! 宏
    ├── lang.rs                           # ui.language/$LANG 解析：auto/en-US/zh-CN/zh-Hant/zh-TW/zh-HK/pseudo
    ├── catalog.rs                        # 嵌入 TOML 加载、flatten、繁中 build 产物和 pseudo 字典装配
    ├── format.rs                         # {placeholder} 替换、抽取和 pseudo 文案扩展
    └── diagnostics.rs                    # 内置 i18n key/placeholder/空值诊断，doctor 复用
```

`assets/themes/*.toml` 是内置主题的唯一事实来源。`build.rs` 在编译期扫描并校验
文件名、显示名、TOML 结构和 palette 引用，然后生成嵌入 binary 的稳定排序 registry；
新增内置主题只需增加一个合法 TOML 文件。

数据流：`daemon::process` 初始化 log/config/i18n/overlay 后，在 tokio 线程运行
`daemon::runtime`。键盘事件由 `platform::daemon::DaemonPlatform` 当前的 macOS
实现启动 CGEventTap（Default 模式，可吞）→ 解码成 4 字节 `RawEvent`（含
`EventKind` + `keycode` + 8-bit `ModMask`）→ pipe →
`daemon::hotkey_input` bridge → mpsc → runtime 内的 `TrackerSet`。
CGEventTap 回调里同步问 `Mutex<Suppressor>` 决定 `CallbackResult::Drop` /
`Keep`，suppress `[hotkey].trigger`，并在 recording task 存活期间 suppress
`[hotkey].cancel`；纯键 / combo 采用 reserved 语义，吞 key 部分的 down + 配对
up（`:double` 的第一次候选也吞，§5 不变量 8 保证 reload 中途换 binding 也安全），
modifier-only 不吞任何事件（modifier 太常用）。`Tracker` 内部分三个
sub-machine：纯键 / combo 走"KeyDown 时 mods 精确匹配 + auto-repeat 去抖"；
modifier-only 走"FlagsChanged 检测 clean tap"（500ms hold 阈值 + 中间无普通键
+ 中间无额外 modifier）；`:double` 后缀在 `register_tap` 用 400ms 窗口判定。

`trigger` 第一次命中 = toggle ON：runtime 通过 `DaemonPlatform::frontmost_app`
取当前 App，上交 `daemon::session_start` 按 `config.toml` 的 `[profile]` 路由选定
profile；profile 的 `[asr]` 决定 ASR provider、hotwords 和 provider 字段覆盖，
`[post].chain` 再引用 `post/rule/*.toml` / `post/llm/*.toml` 组件并应用
`[post.llm.<name>]` 浅覆盖。`session_start` 构造 `SessionParams` 后，runtime
spawn `finish::run_recording` task；若 profile/post/asr 初始化失败，直接通过
i18n 文案发 overlay error，不进入录音 task。`trigger` 第二次命中 = active
session 收到 Stop；cancel hotkey = active session 收到 Cancel，同时 overlay
`Dismiss` 用于清掉 lingering error/notice。录音 task 内部仍由 voice 层完成：
cpal stream → provider session → post chain → dispatch → history → StateStore /
Overlay / UDS fanout。

配置热重载：notify watcher 监听 `~/.config/shuohua/` → 通过 `watch::Sender`
广播给 overlay / i18n / hotkey 三个 subscriber；hotkey subscriber 收到新
trigger/cancel binding 后发给 runtime，runtime 同步替换 `TrackerSet`，并通过
`daemon::hotkey_input` 更新 `Suppressor`。UDS `reload_config` 复用同一个 parse +
broadcast 入口；profile / ASR provider / post components 在下一次录音开始时生效。

## 当前实现状态

`finish::run_recording` 是唯一公开录音入口。`engine::run` 通过
`RecordingMode::{Continuous, VadPause}` 区分固定模式：Continuous 始终向一个
provider session 发送 PCM，不构造 Silero、timeline 或 pre-roll 状态，也不进入
Idle；VadPause 保留 Active / Idle、pause / resume、pre-roll 和 overlap。
engine 返回 `EngineOutcome` 后，finish 统一执行 post/dispatch、history 和最终
StateStore / Overlay completion。
`voice/silero.rs`、`voice/timeline.rs`、`voice/vad.rs` 都是默认 build 编译。
`voice/observer.rs` 是 `feature=dev` 下的 trace observer，默认 build 是 ZST no-op。

`finish.rs` 是 voice 子系统 completion 顶层，依赖
`engine / capture / history_build / post_dispatch`；`engine.rs` 只依赖录音运行期的
`capture / finalize / meter / observer / vad / silero / timeline / recorder / audio`，
不调用 `post::run_chain`、`dispatch::dispatch` 或 history append。
engine 对 `post` 的全部接触面是：用 `post::AppContext` 作为前台 App 上下文的数据
载体、在 stop 时调一次 `post::app_context::frontmost_app()`、以及读
`SessionParams.post_chain.name` 作为 overlay header 的 chain summary 字符串。
`SegmentCapture / SessionCapture` 仅 `pub(crate)` 暴露在 voice 模块内部。

`engine::run` 负责 recorder 启动这一个 `!Send` 边界；其余初始化、ASR
event、stop drain、provider finalize、错误/取消、retained audio 都在
`engine::run_with_recorder` 内部。`#[cfg(test)] RecordingStream::for_test` 让
`voice/engine_lifecycle_tests.rs` 不依赖 cpal 即可驱动整个录音生命周期。

daemon 当前的跨平台边界是轻量 adapter，不是完整平台抽象：`platform::daemon`
只封装 daemon runtime 立刻需要的 `frontmost_app` 和 hotkey event tap 启动。
后续真正做 Linux / Windows 时，再把 hotkey input、overlay runner、single-instance
lock、dispatch/autotype 等 OS 能力逐步纳入平台层；在此之前不提前扩 trait，避免
为未知平台假设过度抽象。

每条路径的详细职责见 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)；关键设计决策见 [DESIGN.md §2](DESIGN.md#2-关键设计决策)。
