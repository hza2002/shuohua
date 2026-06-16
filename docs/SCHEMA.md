# 数据 Schema

所有持久化/线协议数据格式集中在这里：UDS 协议 + history JSONL。

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

状态机当前快照只通过 UDS `subscribe` 拿。**唯一持久化数据集是 history JSONL**；daemon 诊断日志是排障 sidecar，不作为状态源。这样：

- 单一真相来源（history 派生统计）
- 不浪费 SSD（无 1Hz 写）
- daemon 无 TUI 时几乎完全空闲
- 未来 GUI / menubar app 走同一个 UDS + history JSONL，零返工

---

## 2. History JSONL Schema（v2）

history JSONL 是**唯一的持久化数据源**（统计、TUI、未来 GUI/脚本都派生自它）。文件按本地月份分片：

```
${XDG_STATE_HOME:-~/.local/state}/shuohua/history/YYYY-MM.jsonl
```

文件名使用本地月份；每条 record 内部 `started_at` / `ended_at` 仍使用 UTC RFC3339。
**一次 recording = 一条 JSON 行**，pipeline 跑完 dispatch 完成那一刻 append 一次。无
pretty print、UTF-8 无 BOM、`\n` 结尾。record JSON schema 不因分片变化升 version。

v2 是破坏式升级，不兼容 v1。v1 记录在 v2 读侧无法解析；开发阶段不做旧路径兼容或自动迁移。

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
    "duration_ms": 5300,
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
- **`asr.text`**：所有 `sessions[].text` 用 provider 分隔符拼接。单 session 路径下等于 `sessions[0].text`；M10 多 session 启用时等于全部 session 的顺序拼接。
- **`asr.duration_ms`**：ASR 工作窗口 = `sessions[-1].ended_at − sessions[0].started_at`，即"如果不开 idle_pause、走单 session 会喂出去多少音频"的真实基线。空 `sessions[]` 时 = 0。M10 跳过的纯静音时长 = `asr.duration_ms − asr.audio_ms`。**不要**用顶层 `duration_ms` 当分母——后者包含 post chain 等收尾时间。
- **`asr.audio_ms`**：Σ `sessions[].audio_ms`，是 provider 实际收到的音频总时长，也是按音频时长计费 provider 的计费事实。单 session 路径下 `audio_ms == duration_ms`（没有静音被跳过）。
- **`asr.sessions[].audio_ms`**：该段实际喂出去的音频时长，严格等于 `(ended_at − started_at).whole_milliseconds()`。
- **时间戳**：ISO 8601 / RFC 3339，UTC（`Z` 后缀）。
- **`_ms` 后缀**：所有数字时长字段必带，允许浮点（亚毫秒精度）。
- **顶层 `status`**：`submitted | canceled | error | timeout`。
- **pipeline 步骤 `status`**：`ok | error | timeout | skipped`。
- **`error` 字段**：`status != submitted` 时追加 `"error": { "kind": "...", "msg": "..." }`，否则 serde `skip_serializing_if` 省略。
- **`app`**：bundle ID 字符串；取不到为 `null`。

### 2.2 不变量

写入侧必须满足，读侧可断言：

- `duration_ms == (ended_at - started_at).whole_milliseconds()`
  - 注意：`duration_ms` 是 recording 整体壁钟（包含 post chain、history append 等收尾时间），**不**等于 ASR 实际工作窗口。计算"客户端 VAD 省了多少音频时长"时不能用 `duration_ms` 做分母。
- `asr.audio_ms == Σ sessions[].audio_ms`
- `sessions[].audio_ms == (ended_at - started_at).whole_milliseconds()`，即 session 的起止严格等于"首发样本→末发样本"在 recording timeline 上的窗口。
- `asr.duration_ms == sessions[-1].ended_at − sessions[0].started_at`（空 sessions 时 = 0）。
- `asr.duration_ms >= asr.audio_ms`；M10 省下的音频时长 = `asr.duration_ms − asr.audio_ms`。
- 单 session 路径（`idle_pause = false` 或 `voice.vad.backend = "off"`）：`sessions.len() <= 1`，`asr.audio_ms == ASR 工作窗口`（没有跳过）。
- 多 session 路径：`sessions[]` 按 `started_at` 递增；允许相邻 session 因 resume pre-roll overlap 时间重叠（`sessions[i].ended_at > sessions[i+1].started_at` 可能成立，但相邻 session 间 silence 通常远大于 overlap，所以多数情况下顺序仍递增）。
- `asr.text == sessions[].text 用 provider 分隔符拼接`
- `text == pipeline 最后一个 ok step 的 text；pipeline 全跳/为空时 == asr.text`

### 2.3 启用条件

`sessions[]` 在以下条件全部成立时可包含多条：

- `~/.config/shuohua/config.toml` 中 `[voice.vad] backend = "silero"`。
- `~/.config/shuohua/asr/<provider>.toml` 中 `idle_pause = true`。

任何一个不成立时仍按单 session 路径走，`sessions[]` 最多一条。详见 [M10 Multi-session ASR](M10.md)。

### 2.4 空间估算

每条 ~500B 元数据 + text 长度。一天 200 次录音 ≈ 1MB。不需要压缩，后续按月 rotate 再考虑。

### 2.5 消费方契约

版本字段 `version: 2`；增字段不破坏，删字段才升 version。v1 记录（`version: 1`）不再可解析，已清空。

### 2.6 写入失败处理

append 失败（磁盘满 / 权限错 / inode 跑光）时，写 daemon 诊断日志 + 推一条 UDS `error` 事件，但 daemon **不崩**。这条 recording 的数据丢弃，下条继续尝试。罕见路径（磁盘满几乎不会发生），不做自动回收或重试。

---

## 3. 音频文件留存（可选）

`voice.record_audio = true` 时，每次 recording 落一个 WAV：

```
${XDG_STATE_HOME:-~/.local/state}/shuohua/audio/<recording_id>.wav
```

- **`<recording_id>` = history record 的 ULID**。ULID 在 recording 开始时生成；同一 ULID 进 jsonl 的 `id` 字段，wav 跟 history 行天然 join。
- **格式固定**：16kHz s16le mono PCM（canonical 内部格式，跟喂给 ASR 的 PCM 同一份）。
- **一次 recording = 一个 wav**，含静音段。段边界从 `history.asr.sessions[].started_at/ended_at` 时间戳切分（`ffmpeg -ss <start> -t <dur>` 重放某段，零信息丢失）。M10 后相邻 session 可因 pre-roll overlap 重叠；按各自时间戳切分即可复现 provider input。
- **关闭路径 (`record_audio = false`, 默认)** 完全跳过写入逻辑：cpal callback 拿到 PCM 直接喂 ASR，不复制一份到落盘 buffer。
- **写入失败**：跟 history JSONL 同语义——写 daemon 诊断日志 + UDS `error` 事件，daemon 不崩，这次录音不留 wav，下次继续尝试。
- **不存 history 字段**：路径完全由 ULID + 目录约定推出，无需在 history record 加 `audio_path`。文件不存在 = `record_audio` 当时是 false（或写失败）。

---

## 4. VAD Trace（开发期 sidecar）

`voice.vad_trace = true` 且 binary 用 `--features dev` 构建时，每次 recording 额外写：

```
${XDG_STATE_HOME:-~/.local/state}/shuohua/traces/<recording_id>.jsonl
```

这是 M10 VAD 评估用 sidecar，不属于 history schema；默认 build 不写 trace。每行一个 JSON 事件，当前包含：

- `recording_start`：recording id、provider、Silero shadow 参数。
- `vad_frame`：每个 512-sample 窗口的 `start_ms/end_ms/probability/speech`。
- `vad_transition`：shadow controller 预测的 `resume/pause`，含 pre-roll 后的 `at_ms`。
- `provider_opened`：ASR provider open 完成时间。
- `asr_partial` / `asr_segment` / `asr_error` / `asr_done`：ASR 事件时间；Doubao segment 使用服务端 `start_time/end_time`（缺失时回退到收到事件时间）。
- `recording_end`：最终 status、实际喂给 ASR 的 `audio_ms`、shadow VAD 预计 `active_ms/saved_ms/sessions`。

trace 文件可随时删除，不被 TUI/GUI 消费。它只用于离线评估“VAD active 区间是否覆盖 ASR utterance”和“预计省费比例”。
