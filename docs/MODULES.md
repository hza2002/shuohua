# 模块规划

`src/` 只包含已实现的模块。路径细节看 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)，阶段历史看 [CHANGELOG.md](../CHANGELOG.md)。

## 已实现

```
src/
├── main.rs                              # clap 入口；smart fallback；--daemon 跑 AppKit + tokio daemon；F16 toggle 状态机
├── cli/
│   ├── mod.rs                           # clap 子命令分发
│   ├── doctor.rs                        # shuo doctor：本地配置诊断；--runtime 显式跑 ASR/LLM 可运行性检查入口
│   ├── config_template.rs               # shuo config-template：导出 registry 模板
│   └── service.rs                       # launchd install/uninstall/start/stop/restart/status
├── config/
│   ├── mod.rs                           # config module root；top-level config API re-export + submodules
│   ├── main.rs                          # ~/.config/shuohua/config.toml schema/parse/path helpers
│   ├── spec.rs                          # shared field/spec metadata + validation diagnostics
│   ├── schema.rs                        # shared config schema registry + description i18n keys
│   ├── inventory.rs                     # structured Configure/doctor inventory scan
│   ├── diagnostics.rs                   # full-tree local config diagnostics shared by doctor/Configure
│   ├── template.rs                      # official config template registry + LLM component creation
│   ├── profile.rs                       # profile/*.toml schema + route loading
│   ├── post/                            # post component config namespace
│   ├── asr/                             # ASR provider config loaders
│   └── theme.rs                         # reserved theme namespace
├── log.rs                                # tracing 初始化：daily file appender、本地时间格式、TTY mirror
├── reload.rs                             # notify watcher + watch::Sender 广播；overlay/i18n/hotkey subscriber；UDS 手动 reload 复用同一路径
├── clipboard_darwin.rs                  # NSPasteboard 写文本
├── autotype_darwin.rs                   # CGEventPost Cmd+V
├── focused_window_darwin.rs              # CGWindow + AX 拿 focused window 几何，给 overlay 定位
├── app_context_darwin.rs                 # frontmost app bundle id / 名字，给 overlay header 显示
├── hotkey/
│   ├── mod.rs                           # 4 字节 RawEvent 线协议 + 公共类型 re-export
│   ├── combo.rs                         # Combo / ModMatcher / ModMask / Side / ModType + 精确匹配函数
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
│   ├── app_context.rs                   # post 层 AppContext 入口；macOS 复用 app_context_darwin
│   ├── zh_filter.rs                     # ZhFilter
│   └── llm.rs                           # LlmCleanup；OpenAI-compatible / Anthropic native 一次性调用
├── voice/
│   ├── mod.rs
│   ├── recorder.rs                      # cpal 流式：F32 → 16k mono s16le → mpsc + 可选 wav 留存
│   ├── finish.rs                        # 一次录音生命周期：单/多 session 两条主路径 + post pipeline + dispatch
│   ├── meter.rs                         # 从已有 PCM/VAD 流聚合 50ms audio meter，供 UDS/TUI 画 waveform
│   ├── observer.rs                      # dev observer：VAD shadow trace sidecar（feature=dev；默认 ZST no-op）
│   ├── vad.rs                           # VAD frame/state 边界 + speech/silence hysteresis controller
│   ├── silero.rs                        # Silero VAD 帧检测器（M10，默认 build）
│   ├── timeline.rs                      # Sample-indexed PCM ring buffer + slice_from（M10 resume 用）
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
│   ├── audio.rs                         # History retained audio path/status/open/reveal/delete helpers
│   ├── config_actions.rs                # Configure editor/Finder launcher helpers
│   ├── panes.rs                         # 状态、实时文本、pipeline、历史、Configure 渲染
│   ├── keybindings.rs                   # Tab/Shift-Tab + 1/2/3 翻页；vim/方向键滚动
│   └── settings.rs                      # Configure inventory rows；脱敏展示 secret 字段
├── overlay/
│   ├── mod.rs                           # OverlayCmd + OverlayState + OverlayHandle (mpsc 发命令)
│   ├── view.rs                          # AppKit NSGlassEffectView 同进程渲染主循环
│   └── debug.rs   (#[cfg(debug_assertions)])  # NSGlassEffectView SPI 探针
└── i18n/
    └── mod.rs                            # assets/i18n/*.toml 加载 + 静态 LANG 切换
```

数据流：键盘事件 → CGEventTap 回调（Default 模式，可吞）→ 解码成 4 字节 `RawEvent`（含 `EventKind` + `keycode` + 8-bit `ModMask`）→ pipe → mpsc → `trigger_tracker` / `cancel_tracker` 分别 `on_event(ev, Instant::now())` → tokio main loop。回调里同步问 `Mutex<Suppressor>` 决定 `CallbackResult::Drop` / `Keep`，suppress `[hotkey].trigger`，并在 recording task 存活期间 suppress `[hotkey].cancel`；纯键 / combo 采用 reserved 语义，吞 key 部分的 down + 配对 up（`:double` 的第一次候选也吞，§5 不变量 8 保证 reload 中途换 binding 也安全），modifier-only 不吞任何事件（modifier 太常用，吞了破坏太多）。`Tracker` 内部分三个 sub-machine：纯键 / combo 走"KeyDown 时 mods 精确匹配 + auto-repeat 去抖"；modifier-only 走"FlagsChanged 检测 clean tap"（500ms hold 阈值 + 中间无普通键 + 中间无额外 modifier）；`:double` 后缀在 `register_tap` 用 400ms 窗口判定。`trigger` 第一次命中 = toggle ON 瞬间取一次 `frontmost_app`，按 `config.toml` 的 `[profile]` 路由选定 profile；profile 的 `[asr]` 决定 ASR provider、hotwords 和 provider 字段覆盖，`[post].chain` 再引用 `post/rule/*.toml` / `post/llm/*.toml` 组件并应用 `[post.llm.<name>]` 浅覆盖；spawn `finish::run_recording` 任务：cpal stream → `DoubaoSession.send_pcm` 流式推、`AsrEvent::Segment` 累积、`StateStore` 同步状态、`OverlayHandle` 推 UI 命令、UDS server fanout 给 TUI。`trigger` 第二次命中 = oneshot 通知 task 收尾：toggle OFF 瞬间再取一次 `frontmost_app` 只作为 prompt 变量，不重新选择 profile；drain `stop_delay_ms` 尾音 → send `is_last` → 等 Done（provider 私有 `finalize_timeout_ms`，Doubao 默认 12s）→ segments 直接 concat（provider 自带分隔） → 已选定的 post chain（执行时 overlay 显示 Thinking；单步失败/超时跳过 + meta 行 notice 黄字 3s + pipeline trace；致命错误经 text 区 error 红字反馈并跳过 dispatch）→ 剪贴板 + Cmd+V → monthly history JSONL 落一行 → `history_appended` 推给 TUI。TUI 主循环按 `voice::meter::METER_INTERVAL_MS`（50ms）绘制，并在帧间 drain IPC/key event，避免 audio meter 事件堆满 per-client queue；IPC queue full warn 做 1s 节流，只作为异常诊断信号。配置热重载：notify watcher 监听 `~/.config/shuohua/` → 通过 `watch::Sender` 广播给 overlay / i18n / hotkey 三个 subscriber；hotkey subscriber 收到新 trigger/cancel binding 时同步更新两个 `Tracker`，并把 trigger/cancel 写入 `Suppressor`；UDS `reload_config` 复用同一个 parse + broadcast 入口；profile / ASR provider / post components 在下一次录音开始时生效。

## 当前实现状态

M10 多 session ASR 已接入主录音流程。`finish::run_recording` 入口按 `params.idle_pause && params.vad.backend == Silero` 二选一分派到 `run_single_session_recording`（保持 M9 行为）或 `run_multi_session_recording`（Active / Pausing / Idle / Opening 状态机）。`voice/silero.rs`、`voice/timeline.rs`、`voice/vad.rs` 都是默认 build 编译。`voice/observer.rs` 是 `feature=dev` 下的 trace observer，默认 build 是 ZST no-op。`cli/vad_probe.rs` 已删除；离线 threshold 评估改用保留 WAV + trace 后处理脚本。

每条路径的详细职责见 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)；关键设计决策见 [DESIGN.md §2](DESIGN.md#2-关键设计决策)。
