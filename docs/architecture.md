# architecture — 进程模型 · 数据流 · 全局约定

**TL;DR**：单进程 daemon（AppKit 主线程 + 专用热键线程 + tokio runtime）+ 按需连接的 TUI；通过 UDS 流状态、history JSONL 落地；跨模块边界靠 facade + 单向依赖，不抽 Plugin。

> **何时读**：改进程/线程模型、跨模块数据流、选型、错误/日志/i18n/测试/安全约定。
> **不在这里**：UDS/history 格式见 [schema](schema.md)；CLI/launchd 见 [cli](cli.md)；模块内部见 [modules/](modules/)。

## 1. 进程与线程模型

单进程，不拆子进程（NSGlassEffectView 宿主进程内嵌即可）：

| 线程 | 职责 |
|---|---|
| 主线程（AppKit/CFRunLoop） | NSPanel + glass 渲染；`NSApplication.run()` |
| 专用 OS 线程（CFRunLoop） | CGEventTap 热键拦截（callback 不让出） |
| tokio multi-thread runtime | 录音、VAD、ASR、post、UDS server、history、reload |

| 进程 | 何时跑 | 职责 |
|---|---|---|
| daemon（`shuo --daemon` 或 smart fallback） | launchd 开机自启，常驻 | 上述全部 |
| TUI（`shuo` 检测到 daemon 存在） | 用户按需 | 连 UDS 看状态/历史；关掉不影响 daemon |

**Daemon↔TUI 双通道**：UDS `/tmp/shuohua-${UID}.sock`（实时状态 + 控制，TUI 不连时零 UI 开销）+ history JSONL（月分片，外部脚本/未来 GUI 也读）。daemon 日志（日分片）TUI 不读。JSONL 是唯一 durable source of truth；统计、分页、analytics 都由 daemon 内存索引从 JSONL 派生。

## 2. 数据流（一次录音）

`daemon::process` 初始化 log/config/i18n/overlay 后在 tokio 线程跑 `daemon::runtime`。

1. 键盘事件：`platform::daemon` 启 CGEventTap（Default 模式可吞）→ 4 字节 `RawEvent` → pipe → `daemon::hotkey_input` bridge → mpsc → runtime 内 `TrackerSet`。回调同步问 `Mutex<Suppressor>` 决定 Drop/Keep；cancel 键是否吞额外看 overlay 是否在屏——overlay 线程经 `Arc<AtomicBool> overlay_on_screen` 单向发布可见性（wait-free，非 mpsc 例外，只发布不等待），suppressor 在 OS 线程直读，runtime 亦读它决定 cancel 是否发 `Dismiss`。trigger/cancel/resume 三个 binding 同时匹配，优先级为 cancel > resume > toggle。
2. trigger 首次命中 = toggle ON：runtime 取 frontmost app → `daemon::session_start` 按 `[profile]` 路由选 profile → 构造 `SessionParams` → spawn `finish::run_recording`。profile/post/asr 初始化失败直接发 overlay error，不进录音 task。
3. trigger 二次命中 = Stop；cancel hotkey = Cancel：有活动 session 就取消,只要 overlay 在屏（含无 session 的 Error 屏）就发 `Dismiss` 关掉,idle 则放行透传。resume hotkey 只在没有活动 session 时生效：读取最新一条 history，若是可恢复的 `canceled` 或 `asr_timeout` 就带 seed 开新 recording，否则等同开新 recording。
4. 录音 task 内：cpal stream → provider session → post chain → dispatch → history → StateStore/Overlay/UDS fanout（见 [voice](modules/voice.md)）。
5. overlay 交互反向流：AppKit 点击 profile picker → `OverlayAction::BindProfile` → tokio runtime task → `toml_edit` 原子替换 `config.toml` → `reload_now()` 广播；watcher 可能再次捕获 rename，但 subscriber 先 diff。
6. 热重载：notify watcher 监听 `~/.config/shuohua/`，`config.toml`/`theme/*.toml` 触发 parse + watch 广播给 overlay/i18n/hotkey；UDS `reload_config` 复用同一入口（见 [config](modules/config.md)）。

## 3. 模块边界与顶层树

```
src/
├── main.rs        clap 入口：--daemon / 子命令 / smart fallback
├── daemon/        bootstrap + tokio 主循环 + hotkey bridge + session_start
├── cli/           doctor / config-template / launchd service
├── config/        config.toml/profile/asr/post/theme 解析 + 校验 + 模板
├── reload.rs      notify watcher + watch 广播 + 三 subscriber
├── paths.rs       state/history/audio/log/trace 路径
├── platform/      OS 能力 facade（clipboard/autotype/permissions/daemon）+ macos/
├── hotkey/        语法 + tracker 状态机 + suppress + CGEventTap          → modules/hotkey.md
├── asr/           AsrProvider/Session trait + providers/                 → modules/asr.md
├── post/          PostProcessor trait + zh_filter + llm                  → modules/post.md
├── voice/         录音生命周期：engine（运行期）+ finish（收尾）          → modules/voice.md
├── overlay/       平台无关 command/model/layout + macos renderer         → modules/overlay.md
├── history/       JSONL records、分页、统计/analytics、删除、audio 关联
├── state/         StateStore 快照广播（非持久状态源）
├── ipc/           UDS server/client + protocol                          → schema.md
├── tui/           ratatui：Status/History/Configure 三页
├── i18n/          内部 i18n（见 §7）
├── log.rs         tracing 初始化
└── text_stats.rs  UAX #29 word count（history/UDS 共用）
```

**单向依赖原则**：`reload` 依赖各模块对外 API，不被反向 import；`voice::engine` 不调 post chain 执行（`voice::post_dispatch`）/ `voice::dispatch` / history；`voice::finish` 构造 `HistoryRecord` 后只通过 `HistoryService` append；`ipc`/TUI 只能经 UDS 和 `crate::history` facade 访问 history，不直接读写 JSONL store primitives；`platform` 业务层调 facade，macOS 实现在 `platform::macos`，非 macOS 返回明确 unsupported。**不抽 Plugin trait**——voice/overlay/debug 固定模块，直接 `tokio::spawn`，配置变化走 `watch::Receiver<Arc<Config>>` 广播。

### 3.1 HistoryService 与 lazy analytics

`history` owns persisted records、bounded pagination、summary stats、year/month/day analytics、record/audio deletion、retained-audio association。JSONL 仍是唯一持久化数据；内存索引只是从 JSONL 派生的可重建缓存，不落盘。

Daemon startup 只创建 `HistoryService` 和轻量目录 watcher，然后继续 bind IPC / register hotkeys；这个路径不 list/open/scan history shards。TUI startup 也只 `subscribe`，第一次进入 History 页才发送当前页、stats 和 visible analytics 请求。第一个 history 请求在 `spawn_blocking` 中初始化索引；并发 history 请求通过同一个 operations mutex 串行，复用初始化结果。

Watcher 只负责创建 history 目录、监听目录并 mark dirty；callback 不 parse JSONL。实际 reconcile 在 request time 执行：读取前取 metadata fingerprint，bounded scan/read，读取后复查 file set fingerprint，一致才接受；不一致 retry once。fingerprint 是跨平台 metadata identity/change marker（macOS/Linux 用 dev/ino/ctime/mtime/len；其他平台可降级），正常路径不做 content hash。

`HistoryService` 的 append/page/stats/analytics/delete 共用一个 operations mutex。page 持锁完成 reconcile + bounded read，并复查 file set fingerprint；事件在释放 mutex 后 publish，避免 subscriber re-enter deadlock。

External edits 语义是 eventually consistent：watcher 事件或下一次 request-time fingerprint 会修复 missed event；和任意 concurrent external writer 不承诺 linearizable。批量编辑 history JSONL 时建议先停 daemon。

**跨平台**：AppKit/core-graphics/objc2 依赖只挂 macOS target。真做 Linux/Windows 时补各平台 sibling（`_darwin.rs` / `overlay/<platform>/`），在此之前不提前扩大 trait 面。

## 4. 技术选型

| 用途 | crate | 关键理由 |
|---|---|---|
| ObjC 互操作 | `objc2` 0.6 + app-kit/foundation 0.3 | 现役标准 |
| CGEventTap | `core-graphics`(≥0.25) + `core-foundation` | **0.25 `CallbackResult::Drop` 是 suppress 落地依赖** |
| 录音 | `cpal` | 简单，已验证稳定 |
| VAD | Silero via `voice_activity_detector`/ORT | 能量阈值类方案误判高 |
| PCM 通道 | `tokio::sync::mpsc::unbounded` | 已在录音路径验证 |
| 唯一 ID | `ulid` | 26 字符含时序，history record id |
| WebSocket | `tokio-tungstenite` + `native-tls` | Doubao 用；macOS 原生 Security，无 rustls 配置负担 |
| TUI | `ratatui` + `crossterm` | 唯一前台 UI |
| 文件监听 | `notify` | 监听**目录**避免 inode 替换 |
| 取消 | `tokio-util` `CancellationToken` | Doubao / IPC server 用；voice 录音 stop/cancel 走 `SessionControl` 两个终态闩（见 voice.md） |
| 时间戳 | `time` | RFC3339，比 chrono 轻 |
| 错误 | `thiserror`(库) + `anyhow`(main) | 见 §5 |
| 日志 | `tracing` + subscriber + appender | 见 §6 |
| Apple ASR | Swift helper + `build.rs` 编译嵌入 | macOS 26+ 本地流式 |
| LLM | `reqwest` + 手写 client | OpenAI 兼容，不引 sdk |

## 5. 错误约定

库错误用 `thiserror` 结构化（`AsrError::Timeout`/`Auth`/`Network` 等，不用字符串），TUI/doctor 按类型给建议；`main` 用 `anyhow`。不引 `log` facade（避免两套 API）。

## 6. 日志（正式诊断日志）

daemon 业务统一 `tracing` → `~/.local/state/shuohua/logs/shuo-YYYY-MM-DD.log`（本地日期，行带 UTC offset；history 时间戳仍 UTC）。前台 `shuo --daemon` 同时 mirror stderr；launchd 不 mirror，plist stdout/stderr 只兜底极早期失败。

- 等级是内部标签，不暴露配置/环境变量。crate 默认收 DEBUG，依赖收 WARN。
- 只记低频锚点（daemon ready、recording started/ended、config reload、各类异常）。
- **不记**识别正文、clipboard、prompt、hotwords 明细、post 输入输出、可能含正文的 provider 原始响应；**不记**每个 partial/segment/VAD frame/PCM（即使 debug build）。高频/正文观测走 dev sidecar：`voice/observer` 的 VAD/ASR trace（`--features dev` + `dev.vad_trace`），以及 Apple backend 本机诊断（`--features dev` + `dev.apple_backend_trace`：多通道 per-channel 探针等）。
- 日志是诊断 sidecar，不是 session 事实源——事实以 history JSONL 为准。

## 7. i18n

内置 zh-CN / en-US 两份人工文案（`assets/i18n/*.toml`，嵌入 binary），`build.rs` 从 zh-CN 派生 zh-Hant/zh-TW/zh-HK（OpenCC s2t/s2twp/s2hk，保留 `{placeholder}`），另有 pseudo 伪语言暴露 UI 截断/漏翻译。**不引 rust-i18n/fluent/ICU 重型框架**。

- `src/i18n/` 是内部模块，不承诺外部 library 稳定性。调用只用 `t!("overlay.state_recording")` / `t!("notice.step_failed", name=...)` / `tr_lang(lang, key, args)`。
- TOML 可嵌套，加载后 flatten 成 dotted key；**叶子必须 string**，否则 build/test 失败（防误写静默丢 key）。
- 质量门禁：`cargo test` 校验 key 集合一致；`diagnostics::diagnose_embedded()` 查 base/derived/pseudo 的 key/placeholder/空值；`shuo doctor` 复用，有问题 blocking。
- 应用范围：overlay/TUI/doctor 走 i18n；clap help 默认 en-US；daemon 日志固定 en-US；history JSONL 不本地化。

## 8. 测试策略

纯函数与 I/O 边界严格分开：

| 模块 | 单测 | proptest | 集成 |
|---|---|---|---|
| config parse/validate | ✓ | — | — |
| hotkey tracker | ✓ | ✓（配对/suppress 不变量） | — |
| voice 状态机 | ✓（fake provider+recorder） | — | — |
| post chain | ✓（fake processor，失败/超时/skip） | — | — |
| asr doubao | — | — | ✓（真实 ASR，需钥匙） |
| ipc 协议 | ✓（serde round-trip） | — | ✓（spawn daemon+假 TUI） |
| overlay AppKit | — | — | 手测 |

Fake 边界：`FakeAsrProvider`（按 PCM 时长产事件，可注入 error）、`FakePasteboard`、`FakeHotkeyProvider`、CGEventTap pipe 用 `os_pipe` 假事件、`RecordingStream::for_test`（不依赖 cpal 驱动 engine）。

## 9. 安全与隐私

- **配置文件**：v1 不自动创建生效配置，用户自行放置；权限沿用 umask（运行时私有的 lock/socket 除外）。
- **API key**：明文 TOML（仓库放模板，用户填），不写入 history。未来可选 `keychain://` 前缀，v1 不做。
- **日志**：见 §6，不记正文。
- **history JSONL 明文**：用户唯一数据源，按月写 `~/.local/state/shuohua/history/`，v1 不自动清理。
- **音频留存可选**：`record_audio = off`(默认)/`lossless`(FLAC)/`compact`(AAC 32k)；实时只写临时 WAV，停止后 `afconvert` 转换并删临时。失败不留 fallback，UDS error + Notice 提醒但不回滚文本。
- **LLM 隔离**：processor 把识别文本发第三方 API，doctor 启动 warn 一次。LLM HTTP 错误的 message/type/code 进 history/log 用于排查 key/额度/模型名（主流 API 错误 message 是开发者诊断信息，不回显 prompt/正文；非主流网关属其信任边界）。
