# History v2（多 Session 前置基础设施）设计

日期：2026-06-15 起草，2026-06-16 缩窄范围
状态：草案，待用户审阅后转实现计划

## 1. 背景与动机

未来希望让一次 recording 能拆成多段真正的 ASR session：本地静音检测触发关旧 session（停止喂音频，省 Doubao 计费），用户重新开口触发开新 session。这是后续工作。

本次只做**前置 schema 改造**：把 history.jsonl 升到 v2，命名错位修掉、字段语义对齐、最终上屏文本显式存、`sessions[]` 数组结构按多 session 的最终形状预留。当前实现永远只产出一条 session（== 当前 recording 的整段），后续接多 session 时只改 writer，不动 schema、不再升 version。

VAD 控制 provider 启停之前实测效果不佳，需要单独调研，本次不做。

## 2. 设计原则

- **schema 一次到位**：按多 session 最终形态定，未来加入多 session 不再升 version。
- **写侧零 derive**：最终上屏文本显式存，读侧不靠 pipeline 反向找。
- **开发期不留兼容**：v1 数据直接丢，单用户、单机器。
- **字段命名贴语义**：`raw` → `text`、统计字段单一化。

## 3. 范围

包含：
- history schema v2（破坏式升级，丢弃 v1 数据）
- `state/history.rs` 数据结构 + 序列化 + 单测
- `voice/finish.rs` 写入侧改造（顶层 `text` 显式存、sessions[] 包一条、删 segment 维度）
- `text_stats.rs` 删 `chars` 字段、保留 `words`
- TUI / IPC 读侧适配（删 `final_text()` 派生、读 `record.text`，统计列只剩 words）
- SCHEMA.md / MODULES.md 同步
- assets/i18n/*.toml 文案

不包含：
- 本地 VAD / 静音检测
- `AsrProvider` trait 变更（`supports_idle_pause` 等能力声明）
- `voice/idle.rs` 新模块
- `voice/finish.rs` 多 session 主循环
- v1 历史数据迁移
- overlay 视觉反馈调整

## 4. Provider 与 Session 当前行为（不变）

`AsrProvider` / `AsrSession` trait 本次不动。`voice/finish.rs` 主循环维持"一次 recording = 一次 `provider.start()`"。当前实现下：

- `sessions[]` 永远 `len == 1`
- `sessions[0].started_at` ≈ recording 的 `started_at`
- `sessions[0].ended_at` ≈ recording 的 `ended_at` 减掉 dispatch / pipeline 时长
- `sessions[0].audio_ms` ≈ recording 的 `duration_ms` 减去 drain 之外的延迟
- `asr.text == sessions[0].text`、`asr.audio_ms == sessions[0].audio_ms`

未来引入多 session 时：唯一变化是 `sessions[]` 出现多条，`asr.text` 由多段拼接，`asr.audio_ms` 是各段之和。

## 5. History Schema v2

破坏式升级，不兼容 v1。开发期清空 `history.jsonl` 重新开始。

```jsonc
{
  "version": 2,
  "id": "01HXYZABC...",                          // ULID（recording 开始生成；ulid crate v1）
  "started_at": "2026-06-13T12:00:00Z",          // RFC 3339 UTC
  "ended_at":   "2026-06-13T12:00:08Z",
  "duration_ms": 8000,                           // recording 挂钟时长
  "status": "submitted",                         // submitted | canceled | error | timeout
  "app": "com.apple.dt.Xcode",                   // bundle_id，取不到为 null
  "text": "今天天气真好，我们出去走走。",          // dispatch 实际上屏的文本（显式存，不派生）
  "text_stats": { "words": 14 },                 // UAX #29 word boundary count
  "asr": {
    "provider": "doubao",
    "text": "今天天气真好",                       // 所有 sessions[].text 用 provider 分隔符拼接；当前 == sessions[0].text
    "audio_ms": 5300,                            // 实际喂 ASR 的音频总时长 = Σ sessions[].audio_ms
    "sessions": [                                // 当前永远 len == 1；多 session 阶段才会 >1
      {
        "text": "今天天气真好",
        "started_at": "2026-06-13T12:00:00Z",
        "ended_at":   "2026-06-13T12:00:05Z",
        "audio_ms":   5300
      }
    ]
  },
  "pipeline": [
    { "name": "filler",     "status": "ok", "duration_ms": 0.3, "text": "今天天气真好" },
    { "name": "llm_casual", "status": "ok", "duration_ms": 820, "text": "今天天气真好，我们出去走走。" }
  ]
  // status != submitted 时追加 "error": { "kind": "asr_timeout", "msg": "..." }
  // status == submitted 时该字段省略（serde skip_serializing_if）
}
```

### 5.1 相对 v1 的变化

| 变更 | 原因 |
|---|---|
| `version: 1` → `2`，破坏兼容 | 命名错位 + 字段语义重排，开发期无迁移成本 |
| 顶层新增 `text` | 显式存最终上屏文本；不再从 `pipeline` 反向 derive |
| `text_stats.chars` 删除 | 单字段足够，跟 `words` 信息重复 |
| `text_stats.words` 保留 | UAX #29 word boundary，`unicode-segmentation::split_word_bounds`，与 Microsoft Word 中文+英文混合"字数"同算法 |
| `asr.raw` → `asr.text` | "raw" 暗示二进制；实际是文本，命名统一 |
| `asr.audio_ms` 语义 | 改为 Σ sessions[].audio_ms（当前 sessions len=1 时等于单段，无行为差异） |
| `asr.sessions[]` 语义 | 从"provider VAD 切的 segment"改为"真正的 ASR 连接段"；当前永远 1 条，结构按未来多 session 预留 |
| `asr.sessions[].audio_ms` | 新增；该段实际喂出去的音频时长 |
| 原 segment 维度 | 删除；TUI 实时 `segment` / `partial` 事件不变（transient，不进 history） |

### 5.2 不变量

写入侧必须满足，读侧可断言：

- `duration_ms == (ended_at - started_at).whole_milliseconds()`
- `asr.audio_ms == Σ sessions[].audio_ms`
- `asr.audio_ms <= duration_ms`
- `sessions[i].ended_at <= sessions[i+1].started_at`（不重叠、严格递增）
- `asr.text == sessions[].text 用 provider 分隔符拼接`
- `text == pipeline 最后一个 ok step 的 text，pipeline 全跳/为空时 == asr.text`
- **当前阶段额外**：`sessions.len() == 1`；多 session 阶段解除

### 5.3 文本统计算法

`text_stats::compute(text)` 沿用现有 UAX #29 word boundary 切分 + 过滤空白逻辑，**只删 `chars` 字段**：

```rust
pub fn compute(text: &str) -> TextStats {
    TextStats {
        words: count_words(text),   // 已有实现：split_word_bounds + filter whitespace
    }
}
```

标点保留计入，与 SCHEMA v1 行为一致。

### 5.4 显示文案

i18n 层：

- `tui.stats.words.en = "words"`
- `tui.stats.words.zh = "字"`

TUI history pane 只显示 `words` 一列，删除原 `chars` 列。

## 6. 模块改动一览

| 文件 | 改动 |
|---|---|
| `src/state/history.rs` | `HistoryRecord` 按 v2 改：顶层加 `text`，`text_stats` 去 `chars`；`AsrHistory` 加 `text`、删 `raw`；`AsrSessionHistory` 加 `audio_ms`；删 `HistoryRecord::final_text()` 方法（直接读 `text` 字段） |
| `src/voice/finish.rs` | append_history 入口：构造单条 `AsrSessionHistory`（含 audio_ms）、顶层 `text` 从 `run_chain` 返回的 `current.text` 直接拿、删除原来反向遍历 pipeline 找 final text 的逻辑 |
| `src/text_stats.rs` | `TextStats` 结构删 `chars` 字段；`compute` 只算 `words`；单测断言更新 |
| `src/state/mod.rs` | 实时统计 snapshot 删 `chars` 字段（如果有），保留 `words` |
| `src/tui/panes.rs` | history pane 删 chars 列；`record.final_text()` 调用点改 `record.text.as_str()` |
| `src/tui/mod.rs` | 同上，`record.final_text()` → `&record.text` |
| `src/ipc/server.rs` | `record.final_text()` 调用点同上 |
| `assets/i18n/*.toml` | 加/改字数显示文案；删除原 chars 文案 |
| `docs/SCHEMA.md` | §2 整段按 v2 重写；多 session 在 §1 / §2 注脚指向当前 sessions.len()==1，多 session 后续阶段解除 |
| `docs/MODULES.md` | 不需要新增条目（无新文件） |
| `docs/DESIGN.md` | 视情况补一句 "history v2、sessions[] 为多 session 预留" |

**不动的文件**：
- `src/asr/types.rs`、`src/asr/providers/*`（trait 不变）
- `src/post/mod.rs`（pipeline 行为不变）
- `src/voice/recorder.rs`
- `src/overlay/*`

## 7. 测试策略

- **history v2 序列化测**（`src/state/history.rs`）：
  - snapshot 一条完整 record，断言 JSON 结构 + 字段名
  - sessions[] 含一条、audio_ms 字段存在
  - `text` 字段写入正确
  - `error` 字段在 status=submitted 时不序列化、status=error 时存在
- **text_stats 单测**：保留现有 word 计数用例，删 chars 相关断言；命名调整为 `words` only
- **finish 集成测**（如有 FakeProvider 集成测）：
  - 断言 `record.text == pipeline 最后 ok step 的 text`
  - pipeline 全失败时 `record.text == asr.text`
  - empty pipeline 时同上
- **TUI 渲染回归**：手动验证（不上自动化）

## 8. 验证（完成判据）

- `cargo fmt && cargo check && cargo test` 全绿
- 用户用 doubao / apple 各录一段，验证：
  - history.jsonl 新行 `version: 2`
  - `sessions[].len() == 1`、`audio_ms` 字段存在且合理
  - 顶层 `text` 等于 dispatch 实际上屏文本
  - `text_stats.words` 数值合理
  - TUI history pane 渲染正常，字数列显示「字」/ "words"
- pipeline 中故意让某 processor 失败 / 超时：`text` 仍取链路上一步 ok 的 text，行为不退化

## 9. 风险

- **现有读侧调用点遗漏**：`final_text()` 删除后所有调用点必须改读 `record.text`。漏改 → 编译失败（好）；改错（误读 `asr.text`）→ 行为差异需要测试覆盖。
- **i18n 文案碎片**：删除 chars 文案时各语言文件都要改，需 grep 全 sweep。
- **历史数据被弃**：用户清空 `history.jsonl` 后旧统计丢失。开发期已接受。

## 10. 后续（不在本次范围）

- **多 session 主循环**：voice 层在 recording 期间多次 `provider.start()` / `session.finish()`；写入侧填多条 `sessions[]`。schema 不变。
- **VAD 调研**：之前简单 RMS 实测效果不佳，需调研 webrtc-vad / Silero VAD / Apple 自带 SFSpeechRecognizer endpoint detection 等方案，选定后再做静音判定。
- **Provider 能力声明**：`AsrProvider` trait 加 `supports_idle_pause`；Apple 关闭、Doubao 开启。
- **`asr.audio_ms` 不再约等于 `duration_ms`**：多 session 之后差额就是省下的计费时长。
- **IdleConfig 进配置文件**：阈值用户可调。
- **overlay idle 反馈**：用户体感稳定后讨论。
