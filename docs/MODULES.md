# 模块规划

`src/` 只包含已实现的模块。未实现的模块按里程碑列在下方，路径细节看 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)，里程碑定义看 [REQUIREMENTS.md §6](../REQUIREMENTS.md#6-里程碑)。

## 已实现（M6 part 1 完）

> M6 拆两 commit：本节状态对应 **part 1**——CGEventTap suppress 真实生效 + Suppressor 状态机 + proptest 覆盖。part 2 将扩 hotkey 语法支持修饰键组合 / 单按 / 双击，届时再更新本节。

```
src/
├── main.rs                              # clap 入口；smart fallback；--daemon 跑 AppKit + tokio daemon；F16 toggle 状态机
├── cli/
│   ├── mod.rs                           # clap 子命令分发
│   ├── doctor.rs                        # shuo doctor：配置 / hotkey / ASR 配置 / UDS / launchd 检查
│   └── service.rs                       # launchd install/uninstall/start/stop/restart/status
├── config.rs                            # ~/.config/shuohua/config.toml 解析
├── log.rs                                # debug_println! 宏（release no-op，DESIGN §2.13）
├── reload.rs                             # notify watcher + watch::Sender 广播；overlay/i18n/hotkey subscriber；UDS 手动 reload 复用同一路径
├── clipboard_darwin.rs                  # NSPasteboard 写文本
├── autotype_darwin.rs                   # CGEventPost Cmd+V
├── focused_window_darwin.rs              # CGWindow + AX 拿 focused window 几何，给 overlay 定位
├── app_context_darwin.rs                 # frontmost app bundle id / 名字，给 overlay header 显示
├── hotkey/
│   ├── mod.rs                           # RawKey + 4 字节线协议 + Tracker/Suppressor re-export
│   ├── parse.rs                         # "f16" → keycode（M2 仅 F1–F20 单键，M6 part 2 扩组合键）
│   ├── tracker.rs                       # 纯函数状态机（去抖 auto-repeat）
│   ├── suppressor.rs                    # 纯函数 suppress 状态机（down/up 配对吞 + trigger 热替换安全）
│   ├── proptests.rs (#[cfg(test)])      # Tracker + Suppressor 跟参考模型逐步等价的 proptest
│   └── provider_darwin.rs               # CGEventTap (Default 模式) → CFRunLoop → pipe + 真吞事件
├── asr/
│   ├── mod.rs
│   ├── types.rs                         # AsrProvider/Session trait + AsrEvent + AsrError；Partial 尾巴语义
│   ├── fake.rs    (#[cfg(test)])        # 测试用 FakeProvider
│   └── providers/
│       ├── mod.rs
│       └── doubao.rs                    # bigmodel_async WS + 二进制帧 + Partial/Segment 映射 + DriftProbe (debug-only)
├── post/
│   ├── mod.rs                           # PostProcessor trait + PipelineText + run_chain
│   └── filler.rs                        # RuleBasedFiller
├── voice/
│   ├── mod.rs
│   ├── recorder.rs                      # cpal 流式：F32 → 16k mono s16le → mpsc + 可选 wav 留存
│   ├── finish.rs                        # 一次录音的生命周期 + stop_delay drain + filler pipeline + dispatch
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

数据流：F16 → CGEventTap 回调（Default 模式，可吞）→ pipe → mpsc → `Tracker` → tokio main loop。回调里同步问 `Mutex<Suppressor>` 决定 `CallbackResult::Drop` / `Keep`：trigger 的 keydown 被吞，对应 keyup 也被吞（§5 不变量 8），即使 reload 中途改了 trigger，旧物理键的 keyup 仍配对吞掉。第一次按 = spawn `finish::run_recording` 任务：cpal stream → `DoubaoSession.send_pcm` 流式推、`AsrEvent::Segment` 累积、`StateStore` 同步状态、`OverlayHandle` 推 UI 命令、UDS server fanout 给 TUI。第二次 F16 = oneshot 通知 task 收尾：drain `stop_delay_ms` 尾音 → send `is_last` → 等 Done（5s 超时）→ segments 直接 concat（provider 自带分隔） → filler pipeline → 剪贴板 + Cmd+V → `history.jsonl` 落一行 → `history_appended` 推给 TUI。配置热重载：notify watcher 监听 `~/.config/shuohua/` → 通过 `watch::Sender` 广播给 overlay / i18n / hotkey 三个 subscriber；hotkey subscriber 收到新 trigger 时同步换 `Tracker` 和 `Suppressor::set_trigger`；UDS `reload_config` 复用同一个 parse + broadcast 入口；`asr.provider` 在下一次录音开始时重新构建 provider 生效。

## 未实现（按里程碑）

| M | 新增路径 | 主要新依赖 |
|---|---|---|
| **M6 part 2** | `hotkey/{combo,mods}.rs`（重写 `parse.rs` + `tracker.rs` + 扩 `suppressor.rs`） | — |
| **M7** | `post/{llm,app_context}.rs` | reqwest |
| **M8** | `asr/providers/whisper_cpp.rs` | whisper-rs (feature flag) |
| **M9** | `asr/providers/apple_speech.rs` | objc2-speech |

每条路径的详细职责见 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)；关键设计决策见 [DESIGN.md §2](DESIGN.md#2-关键设计决策)。
