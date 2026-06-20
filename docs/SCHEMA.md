# 数据 Schema

所有持久化/线协议数据格式集中在这里：UDS 协议 + history JSONL。

> 配套文档：[DESIGN.md](./DESIGN.md) | [CLI.md](./CLI.md) | [CHANGELOG.md](../CHANGELOG.md)

## 1. UDS 协议

`/tmp/shuohua-${UID}.sock`，line-delimited JSON，事件驱动。daemon 端是 server，TUI / 未来 GUI / 外部脚本都是 client。

### 1.1 TUI → daemon（命令）

```jsonc
{"op":"subscribe"}              // TUI 连上立刻发，daemon 回 snapshot + 流式更新
{"op":"reload_config"}
{"op":"get_history","limit":50,"before":"2026-06-12T22:00:00Z","query":"Rust"}
{"op":"daemon_status"}          // 返回 PID / 启动时间 / 在录音否（shuo status 用）
{"op":"shutdown"}               // shuo stop 用；daemon 正常退出 0，避免 launchd KeepAlive 重启
{"op":"start_recording"}        // 预留：当前 daemon 返回 unsupported
{"op":"stop_recording"}         // 预留：当前 daemon 返回 unsupported
{"op":"cancel_recording"}       // 预留：当前 daemon 返回 unsupported
```

当前 UDS 协议类型预留 start/stop/cancel 录音控制，但 daemon 仍返回
`error(kind="unsupported")`。录音控制当前只走全局 hotkey；TUI/GUI 当前定位是状态、
历史、配置查看与维护工具。

### 1.2 daemon → TUI（事件）

```jsonc
{"event":"snapshot","proto_version":2,"state":"idle","recording":null,"started_at":null,"app":null,"app_name":null,"dur_ms":0,"words":0,"segments":[],"partial":""}   // subscribe 回包第一条，含协议版本
{"event":"state_changed","state":"recording","recording_id":"01HXYZ...","started_at":"..."}
{"event":"app_changed","app":"com.apple.dt.Xcode","app_name":"Xcode"}
{"event":"session_meta","recording_id":"01HXYZ...","meta":{"provider":"doubao","chain":"rule:zh_filter → llm:deepseek","vad":"silero","hotwords":3}}
{"event":"session_phase","recording_id":"01HXYZ...","phase":"idle"} // 多 session 路径下 TUI 子状态：active / idle / stopping
{"event":"stats_changed","dur_ms":3200,"words":32}
{"event":"partial","recording_id":"01HXYZ...","text":"今天天气真"}      // ASR 增量
{"event":"segment","recording_id":"01HXYZ...","text":"今天天气真好。"}    // 已定型文本段
{"event":"audio_meter","recording_id":"01HXYZ...","meter":{"rms":0.12,"peak":0.44,"clipped":false,"vad_probability":0.82,"vad_speech":true}} // 录音中输入电平 + VAD 观测
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
- **`audio_meter` 事件**：只在已有录音 PCM/VAD 流上派生轻量监控数据，供 TUI 画 waveform / VAD activity。daemon 不为 TUI 单独打开麦克风流；UDS 不传原始 PCM。`rms` / `peak` / `vad_probability` 取值范围为 `0.0..=1.0`；`vad_probability` / `vad_speech` 在当前录音路径没有 VAD 时可省略。
- **`session_meta` 事件**：录音开始后推一次本次 session 的静态元数据，供 TUI 在 pipeline 尚未执行时显示完整 ASR provider、post chain、VAD backend 和 hotwords 数量。它不是持久化状态；最终事实仍以 `history_appended.record` 为准。
- **`session_phase` 事件**：TUI 专用的录音内部阶段，不改变顶层 `state_changed.state` 语义。`active` 表示正在把音频送 ASR，`idle` 表示 VAD pause 后麦克风仍在听但 ASR 暂停，`stopping` 表示用户停止后的收尾阶段。
- **`get_history` 分页**：默认 `limit=50`，返回从新到旧。`before` 用 `started_at` RFC3339 时间戳，语义为只返回早于该时间的记录。`query` 是可选关键词过滤；当前内置大小写不敏感 substring，未来如需 regex/fzf 体验由 TUI 层增强。
- **协议版本**：`snapshot` 回包带 `proto_version: 2`。TUI 收到不认识的版本号时报 warning 但继续尝试解析；daemon 单方升级版本时必须同时升级 TUI（同二进制 → 不会错位）。未来加事件类型不破坏，删/改字段升 `proto_version`。

### 1.3 不引入 state.json

状态机当前快照只通过 UDS `subscribe` 拿。**唯一持久化数据集是 history JSONL**；daemon 诊断日志是排障 sidecar，不作为状态源。这样：

- 单一真相来源（history 派生统计）
- 不浪费 SSD（无 1Hz 写）
- daemon 无 TUI 时几乎完全空闲
- 未来 GUI / menubar app 走同一个 UDS + history JSONL，零返工

---

## 2. History JSONL Schema（v1）

history JSONL 是**唯一的持久化数据源**（统计、TUI、未来 GUI/脚本都派生自它）。文件按本地月份分片：

```
${XDG_STATE_HOME:-~/.local/state}/shuohua/history/YYYY-MM.jsonl
```

每条记录先完整序列化，再以单个带换行的 buffer 追加。读取时只容忍进程异常退出留下的、
没有结尾换行且无法解析的最后一行；文件中间或已完整换行的损坏记录仍视为读取错误。

文件名使用本地月份；每条 record 内部 `started_at` / `ended_at` 仍使用 UTC RFC3339。
**一次有可归档内容的 recording = 一条 JSON 行**，pipeline 跑完 dispatch 完成那一刻
append 一次。启动录音前失败、ASR 初始连接失败、麦克风 1s watchdog 判定无有效音频等
没有用户语音内容的早期失败只走 overlay / UDS error / daemon log，不写 history。无 pretty
print、UTF-8 无 BOM、`\n` 结尾。record JSON schema 不因分片变化升 version。

v1 是首次公开发布的 history schema。开发期旧记录不兼容、不迁移，发布前直接清空。

```jsonc
{
  "version": 1,
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
  // status == error | timeout 时追加 "error": { "kind": "asr_timeout", "msg": "..." }
  // status == submitted | canceled | empty 时该字段省略（serde skip_serializing_if）
}
```

### 2.1 字段约定

- **顶层 `text`**：dispatch 实际上屏的文本，显式存储，不从 pipeline 反向 derive。
- **`text_stats.words`**：按最终上屏文本（`text`）统计；UAX #29 word boundary，`unicode-segmentation::split_word_bounds` 过滤空白后计数。标点计入，与 Microsoft Word 中英混合"字数"同算法。
- **`asr.text`**：所有 `sessions[].text` 用 provider 分隔符拼接。Continuous 模式下等于唯一 session 的文本；VadPause 启用时等于全部 session 的顺序拼接。
- **`asr.duration_ms`**：ASR 工作窗口 = `sessions[-1].ended_at − sessions[0].started_at`，即 Continuous 模式会喂出去多少音频的真实基线。空 `sessions[]` 时 = 0。净节省时长按有符号数计算：`net_saved_ms = asr.duration_ms - asr.audio_ms`；正数表示跳过的静音多于 overlap 开销，负数表示 overlap 重复发送开销更大。**不要**用顶层 `duration_ms` 当分母——后者包含 post chain 等收尾时间。
- **`asr.audio_ms`**：Σ `sessions[].audio_ms`，是 provider 实际收到的音频总时长，也是按音频时长计费 provider 的计费事实。Continuous 模式下 `audio_ms == duration_ms`（没有静音被跳过）。
- **`asr.sessions[].audio_ms`**：该段实际喂出去的音频时长，严格等于 `(ended_at − started_at).whole_milliseconds()`。
- **时间戳**：ISO 8601 / RFC 3339，UTC（`Z` 后缀）。
- **`_ms` 后缀**：所有数字时长字段必带，允许浮点（亚毫秒精度）。
- **顶层 `status`**：`submitted | canceled | empty | error | timeout`。
  - `submitted`：有最终文本，并已完成 dispatch。
  - `canceled`：用户主动取消。
  - `empty`：ASR 正常结束但没有识别出文本；跳过 post/dispatch，不写剪贴板。
  - `error`：录音、ASR、dispatch 等失败路径。
  - `timeout`：recording 必经链路超时导致无法完成 dispatch；当前用于 ASR finalize
    超时（`error.kind = "asr_timeout"`）。
- **pipeline 步骤 `status`**：`ok | error | timeout | skipped`。这里的 `timeout`
  表示某个 post processor（包括 LLM processor）单步超时并被跳过；它不会自动把顶层
  `status` 改成 `timeout`。
- **`error` 字段**：`status == error | timeout` 时追加
  `"error": { "kind": "...", "msg": "..." }`；`submitted`、用户主动 `canceled` 和空识别
  `empty` 省略。取消和空识别都不是错误。
- **`app`**：bundle ID 字符串；取不到为 `null`。

### 2.2 不变量

写入侧必须满足，读侧可断言：

- `duration_ms == (ended_at - started_at).whole_milliseconds()`
  - 注意：`duration_ms` 是 recording 整体壁钟（包含 post chain、history append 等收尾时间），**不**等于 ASR 实际工作窗口。计算"客户端 VAD 省了多少音频时长"时不能用 `duration_ms` 做分母。
- `asr.audio_ms == Σ sessions[].audio_ms`
- `sessions[].audio_ms == (ended_at - started_at).whole_milliseconds()`，即 session 的起止严格等于"首发样本→末发样本"在 recording timeline 上的窗口。
- `asr.duration_ms == sessions[-1].ended_at − sessions[0].started_at`（空 sessions 时 = 0）。
- `net_saved_ms = asr.duration_ms as i64 − asr.audio_ms as i64`；允许为负，负值表示 resume overlap 的重复发送开销超过跳过的静音。
- Continuous 模式（`idle_pause = false` 或 `voice.vad.backend = "off"`）：`sessions.len() <= 1`，`asr.audio_ms == ASR 工作窗口`（没有跳过）。
- VadPause 模式：`sessions[]` 按 `started_at` 递增；允许相邻 session 因 resume pre-roll overlap 时间重叠（`sessions[i].ended_at > sessions[i+1].started_at` 可能成立，但相邻 session 间 silence 通常远大于 overlap，所以多数情况下顺序仍递增）。
- `asr.text == sessions[].text 用 provider 分隔符拼接`
- `text == pipeline 最后一个 ok step 的 text；pipeline 全跳/为空时 == asr.text`

### 2.3 启用条件

`sessions[]` 在以下条件全部成立时可包含多条：

- `~/.config/shuohua/config.toml` 中 `[voice.vad] backend = "silero"`。
- `~/.config/shuohua/asr/<provider>.toml` 中 `idle_pause = true`。

任何一个不成立时使用 Continuous 模式，`sessions[]` 最多一条。控制协议见 [DESIGN.md §2.9](DESIGN.md#29-客户端-vad--多段-session思考不计费机制)。

### 2.4 空间估算

每条 ~500B 元数据 + text 长度。一天 200 次录音 ≈ 1MB。不需要压缩，后续按月 rotate 再考虑。

### 2.5 消费方契约

版本字段 `version: 1`；增字段不破坏，删字段或破坏兼容时才升 version。

### 2.6 写入失败处理

append 失败（磁盘满 / 权限错 / inode 跑光）时，写 daemon 诊断日志 + 推一条
`{"event":"error","recording_id":"...","kind":"history_append","msg":"..."}`，同时在
overlay meta 行显示“文本已输出，但历史记录保存失败”的本地化 Notice。dispatch
已经发生时不回滚剪贴板或上屏结果，也不把 daemon 顶层状态改成 Error；Hide 延迟到
Notice TTL 到期。daemon **不崩**，这条 recording 的 history 数据丢弃，下条继续尝试。
罕见路径不做自动回收或重试。

---

## 3. 音频文件留存（可选）

`voice.record_audio` 控制 retained audio：

```
off      → 不保存
lossless → ${XDG_STATE_HOME:-~/.local/state}/shuohua/audio/<recording_id>.flac
compact  → ${XDG_STATE_HOME:-~/.local/state}/shuohua/audio/<recording_id>.m4a
```

- **`<recording_id>` = history record 的 ULID**。ULID 在 recording 开始时生成；history 与音频用同一 ID join。
- **格式**：`lossless` 使用 FLAC 无损；`compact` 使用 AAC-LC 32 kbps，实测约比 FLAC 再省 75% 空间。两者输入都是 recorder 生成的 16kHz mono 16-bit canonical PCM。
- **一次 recording 最多一个音频文件**，含静音段。`.flac` 与 `.m4a` 不应同时存在；TUI 遇到该异常时不擅自选择。
- **收尾转换**：实时 callback 只写 `<recording_id>.tmp.wav`；录音停止并 finalize 后，用 macOS `/usr/bin/afconvert` 转成临时目标，再原子 rename 为最终文件。成功后删除临时 WAV。
- **关闭路径 (`record_audio = "off"`, 默认)** 完全跳过写入逻辑。
- **写入/转换失败**：删除所有临时音频，不保留 WAV fallback；写 daemon 诊断日志 + UDS `error(kind=audio_save)` + overlay Notice。daemon 不崩，不回滚文本 dispatch/history，下次继续尝试。
- **不存 history 字段**：路径由 ULID + 两个合法后缀约定推出，无需 `audio_path`。文件不存在 = 当时关闭、保存失败，或之后被用户删除。
- **删除语义**：TUI 只删除对应 `.flac` 或 `.m4a`，不改 history JSONL。

---

## 4. VAD Trace（开发期 sidecar）

`dev.vad_trace = true` 且 binary 用 `--features dev` 构建时，每次 recording 额外写：

```
${XDG_STATE_HOME:-~/.local/state}/shuohua/traces/<recording_id>.jsonl
```

这是 VAD 评估用 sidecar，不属于 history schema；默认 build 不写 trace。每行一个 JSON 事件，当前包含：

- `recording_start`：recording id、provider、Silero shadow 参数。
- `vad_frame`：每个 512-sample 窗口的 `start_ms/end_ms/probability/speech`。
- `vad_transition`：shadow controller 预测的 `resume/pause`，含 pre-roll 后的 `at_ms`。
- `provider_opened`：ASR provider open 完成时间。
- `asr_partial` / `asr_segment` / `asr_error` / `asr_done`：ASR 事件时间；Doubao segment 使用服务端 `start_time/end_time`（缺失时回退到收到事件时间）。
- `recording_end`：最终 status、实际喂给 ASR 的 `audio_ms`、shadow VAD 预计 `active_ms/saved_ms/sessions`。

trace 文件可随时删除，不被 TUI/GUI 消费。它只用于离线评估“VAD active 区间是否覆盖 ASR utterance”和“预计省费比例”。
