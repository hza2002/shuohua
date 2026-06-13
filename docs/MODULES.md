# 模块规划

`src/` 只包含已实现的模块。未实现的模块按里程碑列在下方，路径细节看 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)，里程碑定义看 [REQUIREMENTS.md §6](../REQUIREMENTS.md#6-里程碑)。

## 已实现（M1）

```
src/
├── main.rs                       # 启动 hotkey 线程 + 主循环跑 Tracker → 触发 recorder
├── hotkey/
│   ├── mod.rs                    # RawKey 类型 + 4 字节线协议 encode/decode
│   ├── tracker.rs                # 纯函数状态机（去抖 auto-repeat），7 个单测
│   └── provider_darwin.rs        # CGEventTap (Session, ListenOnly) → CFRunLoop → pipe
└── voice/
    ├── mod.rs                    # 一次性 3 秒录音入口
    └── recorder.rs               # cpal F32 → linear resample → 16k s16le wav (hound)
```

数据流：F16 → CGEventTap → pipe → `Tracker::on_raw` → `voice::record_three_seconds` → `tmp/m1-N.wav`。

## 未实现（按里程碑）

| M | 新增路径 | 主要新依赖 |
|---|---|---|
| **M2** | `config.rs`, `hotkey/{registry,parse}.rs`, `asr/{mod,types,providers/doubao}.rs`, `voice/{finish,dispatch}.rs`, `autotype_darwin.rs`, `clipboard_darwin.rs` | tokio, tokio-tungstenite, async-trait, toml, serde, arc-swap, phf |
| **M2.5** | `voice/vad.rs`, `post/{mod,filler}.rs`, `i18n/mod.rs`, `assets/i18n/*.toml` | webrtc-vad, rtrb, regex |
| **M3** | `state/{mod,history}.rs`, `overlay/{mod,view,animations}.rs`, `build.rs` 链接 frameworks | objc2-quartz-core, serde_json, ulid, time |
| **M4** | `ipc/{mod,protocol}.rs`, `tui/{mod,panes,keybindings}.rs` | ratatui, crossterm |
| **M5** | `cli/{mod,doctor,service,smart}.rs`, `doctor.rs` | clap, notify |
| **M6** | 扩 `hotkey/`（无新路径） | proptest (dev) |
| **M7** | `post/{llm,app_context}.rs` | reqwest |
| **M8** | `asr/providers/whisper_cpp.rs` | whisper-rs (feature flag) |
| **M9** | `asr/providers/apple_speech.rs` | objc2-speech |

每条路径的详细职责见 [DESIGN.md §4](DESIGN.md#4-目录结构初稿)；关键设计决策见 [DESIGN.md §2](DESIGN.md#2-关键设计决策)。
