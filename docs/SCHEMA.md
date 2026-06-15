# 数据 Schema

所有持久化/线协议数据格式集中在这里：UDS 协议 + history.jsonl。

> 配套文档：[DESIGN.md](./DESIGN.md) | [CLI.md](./CLI.md) | [CHANGELOG.md](../CHANGELOG.md)

## 1. UDS 协议

`/tmp/shuohua-${UID}.sock`，line-delimited JSON，事件驱动。daemon 端是 server，TUI / 未来 GUI / 外部脚本都是 client。

### 1.1 TUI → daemon（命令）

```jsonc
{"op":"subscribe"}              // TUI 连上立刻发，daemon 回 snapshot + 流式更新
{"op":"start_recording"}
{"op":"stop_recording"}
{"op":"cancel_recording"}
{"op":"reload_config"}
{"op":"get_history","limit":50,"before":"2026-06-12T22:00:00Z","query":"Rust"}
{"op":"daemon_status"}          // 返回 PID / 启动时间 / 在录音否（shuo status 用）
```

### 1.2 daemon → TUI（事件）

```jsonc
{"event":"snapshot","proto_version":2,"state":"idle","recording":null,"started_at":null,"app":null,"app_name":null,"dur_ms":0,"words":0,"segments":[],"partial":"","stats":{...}}   // subscribe 回包第一条，含协议版本
{"event":"state_changed","state":"recording","recording_id":"01HXYZ...","started_at":"..."}
{"event":"app_changed","app":"com.apple.dt.Xcode","app_name":"Xcode"}
{"event":"stats_changed","dur_ms":3200,"words":32}
{"event":"partial","recording_id":"01HXYZ...","text":"今天天气真"}      // ASR 增量
{"event":"segment","recording_id":"01HXYZ...","text":"今天天气真好。"}    // 已定型文本段
{"event":"pipeline_step","recording_id":"01HXYZ...","name":"filler","status":"ok","duration_ms":0.3,"text":"..."}
{"event":"history","records":[...]}                         // get_history 回包，从新到旧
{"event":"config_reloaded","path":"/Users/me/.config/shuohua/config.toml"} // reload_config 成功回包
{"event":"error","recording_id":"01HXYZ...","kind":"asr_timeout","msg":"..."}
{"event":"history_appended","record":{...}}              // 唯一的"会话完成"事件，含整条 history record
```

**关键约定**：

- **没有独立的 `final` 事件**——会话完成统一通过 `history_appended` 推送整条记录。TUI 想显示"最终上屏文本"就读 `record.text`。
- **`segment` + `partial` 模型**：`segment` 是已定型文本段，`partial` 是当前 utterance 尾巴，会被后续 partial 覆盖。`snapshot.segments` 包含订阅时已经定型的段。TUI 渲染实时文本 = `segments.join("") + partial`，和 overlay 保持一致。
- **`words` 是 shuohua 语义词数**：基于 Unicode word boundary（UAX #29），`unicode-segmentation::split_word_bounds` 过滤空白边界段后计数。英文连续词算 1，中文单字通常算 1，标点算 1，空白算 0。它不是 LLM token count。
- **`pipeline_step` 事件**：让 TUI 能实时看到每个 processor 的产出（流水线观测）。
- **`get_history` 分页**：默认 `limit=50`，返回从新到旧。`before` 用 `started_at` RFC3339 时间戳，语义为只返回早于该时间的记录。`query` 是可选关键词过滤；M4 内置大小写不敏感 substring，未来如需 regex/fzf 体验由 TUI 层增强。
- **协议版本**：`snapshot` 回包带 `proto_version: 2`。TUI 收到不认识的版本号时报 warning 但继续尝试解析；daemon 单方升级版本时必须同时升级 TUI（同二进制 → 不会错位）。未来加事件类型不破坏，删/改字段升 `proto_version`。

### 1.3 不引入 state.json

状态机当前快照只通过 UDS `subscribe` 拿。**唯一持久化文件是 history.jsonl**（外加 release 跑 launchd 时的 stderr 兜底文件，见 [DESIGN.md §2.13](DESIGN.md#213-日志门禁release-vs-debug)）。这样：

- 单一真相来源（history 派生统计）
- 不浪费 SSD（无 1Hz 写）
- daemon 无 TUI 时几乎完全空闲
- 未来 GUI / menubar app 走同一个 UDS + history.jsonl，零返工

---

## 2. History JSONL Schema（v2）

`history.jsonl` 是**唯一的持久化数据源**（统计、TUI、未来 GUI/脚本都派生自它）。**一次 recording = 一条 JSON 行**，pipeline 跑完 dispatch 完成那一刻 append 一次。无 pretty print、UTF-8 无 BOM、`\n` 结尾。

v2 是破坏式升级，不兼容 v1。v1 记录在 v2 读侧无法解析，升级时直接清空 `history.jsonl`。

```jsonc
{
  "version": 2,
  "id": "01HXYZABC...",
  "started_at": "2026-06-13T12:00:00Z",
  "ended_at":   "2026-06-13T12:00:08Z",
  "duration_ms": 8000,
  "status": "submitted",
  "app": "com.apple.dt.Xcode",
  "text": "今天天气真好，我们出去走走。",
  "text_stats": { "words": 14 },
  "asr": {
    "provider": "doubao",
    "text": "今天天气真好",
    "audio_ms": 5300,
    "sessions": [
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

### 2.1 字段约定

- **顶层 `text`**：dispatch 实际上屏的文本，显式存储，不从 pipeline 反向 derive。
- **`text_stats.words`**：按最终上屏文本（`text`）统计；UAX #29 word boundary，`unicode-segmentation::split_word_bounds` 过滤空白后计数。标点计入，与 Microsoft Word 中英混合"字数"同算法。
- **`asr.text`**：所有 `sessions[].text` 用 provider 分隔符拼接；当前 sessions 永远 len=1，等于 `sessions[0].text`。
- **`asr.audio_ms`**：Σ `sessions[].audio_ms`，是实际计费时长。`duration_ms - audio_ms` = client VAD 省下的时长。
- **`asr.sessions[].audio_ms`**：该段实际喂出去的音频时长，是该段的计费贡献。
- **时间戳**：ISO 8601 / RFC 3339，UTC（`Z` 后缀）。
- **`_ms` 后缀**：所有数字时长字段必带，允许浮点（亚毫秒精度）。
- **顶层 `status`**：`submitted | canceled | error | timeout`。
- **pipeline 步骤 `status`**：`ok | error | timeout | skipped`。
- **`error` 字段**：`status != submitted` 时追加 `"error": { "kind": "...", "msg": "..." }`，否则 serde `skip_serializing_if` 省略。
- **`app`**：bundle ID 字符串；取不到为 `null`。

### 2.2 不变量

写入侧必须满足，读侧可断言：

- `duration_ms == (ended_at - started_at).whole_milliseconds()`
- `asr.audio_ms == Σ sessions[].audio_ms`
- `asr.audio_ms <= duration_ms`
- `sessions[i].ended_at <= sessions[i+1].started_at`（不重叠、严格递增）
- `asr.text == sessions[].text 用 provider 分隔符拼接`
- `text == pipeline 最后一个 ok step 的 text；pipeline 全跳/为空时 == asr.text`
- **当前阶段额外**：`sessions.len() == 1`（多 session 阶段后续解除）

### 2.3 当前阶段限制

`sessions[]` 目前永远只有一条（`len == 1`），对应整段 recording。多 session 阶段（VAD 切段、多次 provider.start()）落地后，writer 侧填多条，schema 不再变动。详见设计文档 [`docs/superpowers/specs/2026-06-15-multi-session-asr-and-history-v2-design.md`](superpowers/specs/2026-06-15-multi-session-asr-and-history-v2-design.md)。

### 2.4 空间估算

每条 ~500B 元数据 + text 长度。一天 200 次录音 ≈ 1MB。不需要压缩，后续按月 rotate 再考虑。

### 2.5 消费方契约

版本字段 `version: 2`；增字段不破坏，删字段才升 version。v1 记录（`version: 1`）不再可解析，已清空。

### 2.6 写入失败处理

append 失败（磁盘满 / 权限错 / inode 跑光）时，记录 `eprintln!` + 推一条 UDS `error` 事件，但 daemon **不崩**。这条 recording 的数据丢弃，下条继续尝试。罕见路径（磁盘满几乎不会发生），不做自动回收或重试。

---

## 3. 音频文件留存（可选）

`voice.record_audio = true` 时，每次 recording 落一个 WAV：

```
${XDG_STATE_HOME:-~/.local/state}/shuohua/audio/<recording_id>.wav
```

- **`<recording_id>` = history.jsonl 那条记录的 ULID**。ULID 在 recording 开始时生成；同一 ULID 进 jsonl 的 `id` 字段，wav 跟 history 行天然 join。
- **格式固定**：16kHz s16le mono PCM（canonical 内部格式，跟喂给 ASR 的 PCM 同一份）。
- **一次 recording = 一个 wav**，含静音段。段边界从 `history.asr.sessions[].started_at/ended_at` 时间戳切分（`ffmpeg -ss <start> -t <dur>` 重放某段，零信息丢失）。
- **关闭路径 (`record_audio = false`, 默认)** 完全跳过写入逻辑：cpal callback 拿到 PCM 直接喂 ASR，不复制一份到落盘 buffer。
- **写入失败**：跟 history.jsonl 同语义——`eprintln!` + UDS `error` 事件，daemon 不崩，这次录音不留 wav，下次继续尝试。
- **不存 history 字段**：路径完全由 ULID + 目录约定推出，无需在 history.jsonl 加 `audio_path`。文件不存在 = `record_audio` 当时是 false（或写失败）。
