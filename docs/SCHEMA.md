# 数据 Schema

所有持久化/线协议数据格式集中在这里：UDS 协议 + history.jsonl。

> 配套文档：[REQUIREMENTS.md](../REQUIREMENTS.md) | [DESIGN.md](./DESIGN.md) | [CLI.md](./CLI.md)

## 1. UDS 协议

`/tmp/shuohua-${UID}.sock`，line-delimited JSON，事件驱动。daemon 端是 server，TUI / 未来 GUI / 外部脚本都是 client。

### 1.1 TUI → daemon（命令）

```jsonc
{"op":"subscribe"}              // TUI 连上立刻发，daemon 回 snapshot + 流式更新
{"op":"start_recording"}
{"op":"stop_recording"}
{"op":"cancel_recording"}
{"op":"reload_config"}
{"op":"get_history","limit":50,"before":"2026-06-12T22:00:00Z"}
{"op":"daemon_status"}          // 返回 PID / 启动时间 / 在录音否（shuo status 用）
```

### 1.2 daemon → TUI（事件）

```jsonc
{"event":"snapshot","proto_version":1,"state":"idle","recording":null,"stats":{...}}   // subscribe 回包第一条，含协议版本
{"event":"state_changed","state":"recording","recording_id":"01HXYZ...","started_at":"..."}
{"event":"partial","recording_id":"01HXYZ...","text":"今天天气真"}      // ASR 增量
{"event":"pipeline_step","recording_id":"01HXYZ...","name":"filler","status":"ok","duration_ms":0.3,"text":"..."}
{"event":"error","recording_id":"01HXYZ...","kind":"asr_timeout","msg":"..."}
{"event":"history_appended","record":{...}}              // 唯一的"会话完成"事件，含整条 history record
```

**关键约定**：

- **没有独立的 `final` 事件**——会话完成统一通过 `history_appended` 推送整条记录。TUI 想显示"最终上屏文本"就读 `record.pipeline.last().text`（chain 空时读 `record.asr.raw`）。
- **`pipeline_step` 事件**：让 TUI 能实时看到每个 processor 的产出（流水线观测）。
- **协议版本**：`snapshot` 回包带 `proto_version: 1`。TUI 收到不认识的版本号时报 warning 但继续尝试解析；daemon 单方升级版本时必须同时升级 TUI（同二进制 → 不会错位）。未来加事件类型不破坏，删/改字段升 `proto_version`。

### 1.3 不引入 state.json

状态机当前快照只通过 UDS `subscribe` 拿。**唯一持久化文件是 history.jsonl + log.jsonl**。这样：

- 单一真相来源（history 派生统计）
- 不浪费 SSD（无 1Hz 写）
- daemon 无 TUI 时几乎完全空闲
- 未来 GUI / menubar app 走同一个 UDS + history.jsonl，零返工

---

## 2. History JSONL Schema（v1）

`history.jsonl` 是**唯一的持久化数据源**（统计、TUI、未来 GUI/脚本都派生自它）。**一次 recording = 一条 JSON 行**，pipeline 跑完 dispatch 完成那一刻 append 一次。无 pretty print、UTF-8 无 BOM、`\n` 结尾。

```jsonc
{
  "version": 1,
  "id": "01HXYZABC...",                             // ULID，26 字符
  "started_at": "2026-06-13T12:00:00Z",
  "ended_at":   "2026-06-13T12:00:08Z",
  "duration_ms": 8000,                              // recording 总时长（=ended_at - started_at）
  "status": "submitted",                            // submitted | canceled | error | timeout
  "app": "com.apple.dt.Xcode",                      // bundle_id 字符串；取不到为 null
  "asr": {
    "provider": "doubao",
    "raw": "今天天气真好 我们出去走走",                // 所有 sessions 拼起来的原始文本
    "audio_ms": 5300,                               // 实际喂给 ASR 的音频毫秒数 = 真实计费时长
    "sessions": [                                   // 每个 ASR session 一条（VAD 切的段落）
      { "text": "今天天气真好", "started_at": "2026-06-13T12:00:00Z", "ended_at": "2026-06-13T12:00:03Z" },
      { "text": "我们出去走走", "started_at": "2026-06-13T12:00:05Z", "ended_at": "2026-06-13T12:00:08Z" }
    ]
  },
  "pipeline": [                                     // 每个 processor 一条
    { "name": "filler",     "status": "ok", "duration_ms": 0.3, "text": "今天天气真好 我们出去走走" },
    { "name": "llm_casual", "status": "ok", "duration_ms": 820, "text": "今天天气真好，我们出去走走。" }
  ]
}
```

### 2.1 字段约定

- **顶层 `status`**：录音整体结局。`submitted | canceled | error | timeout`
- **pipeline 步骤 `status`**：单步结果。`ok | error | timeout | skipped`（同字段名，按层级语境）
- **`error` 字段**：`status != submitted` 时追加 `"error": { "kind": "asr_timeout", "msg": "..." }`，其他情况省略（serde `skip_serializing_if`）
- **时间戳**：ISO 8601 / RFC 3339，UTC（`Z` 后缀）
- **`_ms` 后缀**：所有数字时长字段必带，允许浮点（亚毫秒精度）
- **最终上屏文本** = `pipeline.last().text`；chain 空则 = `asr.raw`。**不存独立字段**（避免冗余）
- **`asr.audio_ms`**：实际喂出去的音频时长。`duration_ms - audio_ms` = client VAD 省下的时长 × 3.5 元/h ≈ 这条省了多少钱

### 2.2 空间估算

每条 ~500B 元数据 + text 长度。一天 200 次录音 ≈ 1MB。不需要压缩，v2 再考虑按月 rotate。

### 2.3 消费方契约

版本字段 `version: 1`；增字段不破坏，删字段才升 version。

### 2.4 写入失败处理

append 失败（磁盘满 / 权限错 / inode 跑光）时，记录 `tracing::error!` + 推一条 UDS `error` 事件，但 daemon **不崩**。这条 recording 的数据丢弃，下条继续尝试。罕见路径（磁盘满几乎不会发生），不做自动回收或重试。
