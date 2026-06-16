# 模块规划

`src/` 只包含已实现的模块。路径细节看 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)，阶段历史看 [CHANGELOG.md](../CHANGELOG.md)。

## 已实现

```
src/
├── main.rs                              # clap 入口；smart fallback；--daemon 跑 AppKit + tokio daemon；F16 toggle 状态机
├── app_profile.rs                       # apps/default + apps/<bundle_id> profile 加载（ASR/post 组合 + provider 覆盖）
├── cli/
│   ├── mod.rs                           # clap 子命令分发
│   ├── doctor.rs                        # shuo doctor：配置 / hotkey / ASR 配置 / UDS / launchd 检查
│   ├── vad_probe.rs                     # M10 dev-only Silero WAV fixture probe（feature=dev-vad-probe）
│   └── service.rs                       # launchd install/uninstall/start/stop/restart/status
├── config.rs                            # ~/.config/shuohua/config.toml 解析
├── log.rs                                # debug_println! 宏（release no-op，DESIGN §2.13）
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
│   ├── config.rs                        # post component 加载；app profile 可浅覆盖 LLM component 字段
│   ├── filler.rs                        # RuleBasedFiller
│   └── llm.rs                           # LlmCleanup；OpenAI-compatible / Anthropic native 一次性调用
├── voice/
│   ├── mod.rs
│   ├── recorder.rs                      # cpal 流式：F32 → 16k mono s16le → mpsc + 可选 wav 留存
│   ├── finish.rs                        # 一次录音的生命周期 + stop_delay drain + post pipeline + dispatch
│   ├── trace.rs                         # dev-only VAD shadow trace sidecar（feature=dev-vad-trace；默认 no-op）
│   ├── vad.rs                           # VAD frame/state 边界 + speech/silence hysteresis controller（暂未接入主流程）
│   └── dispatch.rs                      # 剪贴板 + 可选 Cmd+V
├── state/
│   ├── mod.rs                           # StateStore + 原子 subscribe_with_snapshot + StateEvent broadcast
│   └── history.rs                       # history.jsonl append-only writer（schema 见 SCHEMA.md §2）
├── ipc/
│   ├── mod.rs                           # IPC 子模块
│   ├── protocol.rs                      # line-delimited JSON over UDS；Command/Event serde schema
│   ├── server.rs                        # UnixListener；subscribe snapshot；per-client fanout；history 查询
│   └── client.rs                        # TUI/smart fallback 共用 UnixStream framing helper
├── tui/
│   ├── mod.rs                           # ratatui 主循环；Status/History/Settings 三页
│   ├── panes.rs                         # 状态、实时文本、pipeline、历史、配置浏览渲染
│   └── keybindings.rs                   # Tab/Shift-Tab + 1/2/3 翻页；vim/方向键滚动
├── overlay/
│   ├── mod.rs                           # OverlayCmd + OverlayState + OverlayHandle (mpsc 发命令)
│   ├── view.rs                          # AppKit NSGlassEffectView 同进程渲染主循环
│   ├── animations.rs                    # 状态点 / 高度 / 阴影动画曲线
│   └── debug.rs   (#[cfg(debug_assertions)])  # NSGlassEffectView SPI 探针
└── i18n/
    └── mod.rs                            # assets/i18n/*.toml 加载 + 静态 LANG 切换
```

数据流：键盘事件 → CGEventTap 回调（Default 模式，可吞）→ 解码成 4 字节 `RawEvent`（含 `EventKind` + `keycode` + 8-bit `ModMask`）→ pipe → mpsc → `Tracker::on_event(ev, Instant::now())` → tokio main loop。回调里同步问 `Mutex<Suppressor>` 决定 `CallbackResult::Drop` / `Keep`，按 trigger 类型分发：纯键 / combo 吞 key 部分的 down + 配对 up（§5 不变量 8，即使 reload 中途换 trigger 也安全）；modifier-only trigger 不吞任何事件（modifier 太常用，吞了破坏太多）。`Tracker` 内部分三个 sub-machine：纯键 / combo 走"KeyDown 时 mods 精确匹配 + auto-repeat 去抖"；modifier-only 走"FlagsChanged 检测 clean tap"（500ms hold 阈值 + 中间无普通键 + 中间无额外 modifier）；`:double` 后缀在 `register_tap` 用 400ms 窗口判定。第一次 trigger 命中 = toggle ON 瞬间取一次 `frontmost_app`，按 `apps/<bundle_id>.toml` / `apps/default.toml` 选定 app profile；profile 的 `[asr]` 决定 ASR provider、hotwords 和 provider 字段覆盖，`[post].chain` 再引用 `post/rules/*.toml` / `post/llm/*.toml` 组件并应用 `[post.llm.<name>]` 浅覆盖；spawn `finish::run_recording` 任务：cpal stream → `DoubaoSession.send_pcm` 流式推、`AsrEvent::Segment` 累积、`StateStore` 同步状态、`OverlayHandle` 推 UI 命令、UDS server fanout 给 TUI。第二次命中 = oneshot 通知 task 收尾：toggle OFF 瞬间再取一次 `frontmost_app` 只作为 prompt 变量，不重新选择 profile；drain `stop_delay_ms` 尾音 → send `is_last` → 等 Done（5s 超时）→ segments 直接 concat（provider 自带分隔） → 已选定的 post chain（执行时 overlay 显示 Thinking；单步失败/超时跳过 + meta 行 notice 黄字 3s + pipeline trace；致命错误经 text 区 error 红字反馈并跳过 dispatch）→ 剪贴板 + Cmd+V → `history.jsonl` 落一行 → `history_appended` 推给 TUI。配置热重载：notify watcher 监听 `~/.config/shuohua/` → 通过 `watch::Sender` 广播给 overlay / i18n / hotkey 三个 subscriber；hotkey subscriber 收到新 `Combo` 时同步调 `Tracker::set_trigger` 和 `Suppressor::set_trigger`；UDS `reload_config` 复用同一个 parse + broadcast 入口；app profile / ASR provider / post components 在下一次录音开始时生效。

## 当前未实现

M10 多 session ASR 尚未接入主录音流程。当前已存在的 `voice/vad.rs`、`voice/trace.rs`、`cli/vad_probe.rs` 是 M10 前置验证支撑，均为纯 controller 或 feature-gated dev 工具；正式控制协议见 [M10](M10.md)。

每条路径的详细职责见 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)；关键设计决策见 [DESIGN.md §2](DESIGN.md#2-关键设计决策)。
