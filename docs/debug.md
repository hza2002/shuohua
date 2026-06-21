# debug — 排障流程

**TL;DR**：先复现定位，再按"模块文档 → history → 日志 →（必要时）音频 → dev trace"逐层下探；改完按 CLAUDE 验证清单跑。

> 路径/字段不在这里复述，只给指针：格式见 [schema](schema.md)，CLI 见 [cli](cli.md)。

## 顺序

1. **复现 + 定位最小范围**。处理 bug 先复现，别顺手重构。
2. **查模块文档**：按 [CLAUDE.md 路由表](../CLAUDE.md) 找对应模块的不变量/边界——很多"bug"其实是踩了不变量。
3. **看 history**：`~/.local/state/shuohua/history/YYYY-MM.jsonl`（字段含 `status`/`error.kind`/`asr.sessions[]`/`pipeline[]`，见 [schema §2](schema.md)）。一次录音 = 一行，能还原 ASR/post/dispatch 整条结果。
4. **看 daemon 日志**：`~/.local/state/shuohua/logs/shuo-YYYY-MM-DD.log`（低频诊断锚点，见 [architecture](architecture.md) 日志节）。前台 `shuo --daemon` 会同时 mirror 到 stderr。**日志不记识别正文/高频事件**，正文事实以 history 为准。
5. **必要时听音频**：`~/.local/state/shuohua/audio/<id>.flac|.m4a`（需 `voice.record_audio ≠ off`；`<id>` = history ULID，见 [schema §3](schema.md)）。判断是录音问题还是识别问题。
6. **深入 VAD/ASR 时序**：`--features dev` 构建 + `config.toml` 设 `dev.vad_trace = true` → 每次录音写 `~/.local/state/shuohua/traces/<id>.jsonl`（VAD frame/transition、ASR event 时间、session 切分，见 [schema §4](schema.md)）。用于离线评估 pause/resume 切分质量。trace 可随时删，不被 TUI 消费。
7. **改完验证**：`cargo fmt && cargo check && cargo test`；macOS 权限/录音/上屏由用户手测。

## 常见定位捷径

- 尾字被切 → voice 不变量 #3（drain + stop_delay）。
- 浮层材质不对/不显示 → overlay 不变量 #5、macOS 26 fallback。
- 前台 App modifier 卡住 → hotkey 不变量 #7（down/up 配对吞）。
- 录音无声但不报错 → voice 不变量 #12（1s 首帧 watchdog）。
- 半成品上屏/该上没上 → voice 不变量 #13（error/timeout 不 dispatch）。
- 合盖/换设备后异常 → 麦克风 watchdog；看 trace 首帧。
