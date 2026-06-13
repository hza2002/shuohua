# 模块规划

`src/` 只包含已实现的模块。未实现的模块按里程碑列在下方，路径细节看 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)，里程碑定义看 [REQUIREMENTS.md §6](../REQUIREMENTS.md#6-里程碑)。

## 已实现（M2）

```
src/
├── main.rs                              # tokio multi-thread runtime；F16 toggle 状态机
├── config.rs                            # ~/.config/shuohua/config.toml 解析
├── clipboard_darwin.rs                  # NSPasteboard 写文本
├── autotype_darwin.rs                   # CGEventPost Cmd+V
├── hotkey/
│   ├── mod.rs                           # RawKey + 4 字节线协议
│   ├── parse.rs                         # "f16" → keycode（M2 仅 F1–F20 单键）
│   ├── tracker.rs                       # 纯函数状态机（去抖 auto-repeat）
│   └── provider_darwin.rs               # CGEventTap → CFRunLoop → pipe
├── asr/
│   ├── mod.rs
│   ├── types.rs                         # AsrProvider/Session trait + AsrEvent + AsrError
│   ├── fake.rs    (#[cfg(test)])        # 测试用 FakeProvider
│   └── providers/
│       ├── mod.rs
│       └── doubao.rs                    # bigmodel_async WS + 二进制帧 + Partial/Segment 映射
└── voice/
    ├── mod.rs
    ├── recorder.rs                      # cpal 流式：F32 → 16k mono s16le → mpsc + 可选 wav 留存
    ├── finish.rs                        # 一次录音的生命周期 + stop_delay drain + dispatch
    └── dispatch.rs                      # 剪贴板 + 可选 Cmd+V
```

数据流：F16 → CGEventTap → pipe → mpsc → `Tracker` → tokio main loop。第一次按 = spawn `finish::run_recording` 任务：cpal stream → `DoubaoSession.send_pcm` 流式推、`AsrEvent::Partial/Segment` 累积。第二次 F16 = oneshot 通知 task 收尾：drain `stop_delay_ms` 尾音 → send `is_last` → 等 Done（5s 超时）→ 拼最终文本 → 剪贴板 + Cmd+V。32 单测。

## 未实现（按里程碑）

| M | 新增路径 | 主要新依赖 |
|---|---|---|
| **M2.5** | `voice/vad.rs`, `post/{mod,filler}.rs`, `i18n/mod.rs`, `assets/i18n/*.toml` | webrtc-vad, rtrb, regex |
| **M3** | `state/{mod,history}.rs`, `overlay/{mod,view,animations}.rs`, `build.rs` 链接 frameworks | objc2-quartz-core, serde_json, ulid, time |
| **M4** | `ipc/{mod,protocol}.rs`, `tui/{mod,panes,keybindings}.rs` | ratatui, crossterm |
| **M5** | `cli/{mod,doctor,service,smart}.rs`, `doctor.rs` | clap, notify |
| **M6** | 扩 `hotkey/`（无新路径） | proptest (dev) |
| **M7** | `post/{llm,app_context}.rs` | reqwest |
| **M8** | `asr/providers/whisper_cpp.rs` | whisper-rs (feature flag) |
| **M9** | `asr/providers/apple_speech.rs` | objc2-speech |

每条路径的详细职责见 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)；关键设计决策见 [DESIGN.md §2](DESIGN.md#2-关键设计决策)。
