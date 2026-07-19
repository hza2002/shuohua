# 数据 Schema

所有持久化/线协议数据格式集中在这里：UDS 协议 + history JSONL。

> 配套文档：[architecture.md](./architecture.md) | [cli.md](./cli.md) | [CHANGELOG.md](../CHANGELOG.md)

## 1. UDS 协议

`/tmp/shuohua-${UID}.sock`，line-delimited JSON，事件驱动。daemon 端是 server，TUI / 未来 GUI / 外部脚本都是 client。

### 1.1 TUI → daemon（命令）

```jsonc
{"op":"subscribe"}              // TUI 连上立刻发，daemon 回 snapshot + 流式更新
{"op":"reload_config"}
{"op":"get_history","limit":50,"before":"2026-06-12T22:00:00Z","before_id":"01HXYZ...","query":"Rust"}
{"op":"get_history_stats"}
{"op":"get_history_analytics","period":"month","anchor":"2026-06"}
{"op":"get_history_analytics","period":"last_30_days","anchor":"2026-07-08"}
{"op":"delete_audio","id":"01HXYZ..."}
{"op":"delete_history","id":"01HXYZ..."}
{"op":"preview_history_cleanup","filter":{"scope":"audio_only","window":{"older_than_days":30}}}  // 批量清理预览；scope 取 audio_only|record_and_audio；window 取 "all"|{"last_hours":h}|{"last_days":d}|{"older_than_days":n}|{"range":{"from":"YYYY-MM-DD","to":"YYYY-MM-DD"}}
{"op":"execute_history_cleanup","filter":{"scope":"record_and_audio","window":{"older_than_days":30}},"ids":["01HXYZ..."]}  // 删除 preview 快照里这批目标；execute 不重新扫描 filter
{"op":"daemon_status"}          // 返回 PID / 启动时间 / 在录音否（shuo service status 用）
{"op":"shutdown"}               // shuo service stop 用；daemon 正常退出 0，避免 launchd KeepAlive 重启
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
{"event":"session_meta","recording_id":"01HXYZ...","meta":{"provider":"doubao","chain":"zh_filter → deepseek","vad":"silero","hotwords":3}}
{"event":"session_phase","recording_id":"01HXYZ...","phase":"idle"} // 多 session 路径下 TUI 子状态：active / idle / stopping
{"event":"stats_changed","dur_ms":3200,"words":32}
{"event":"partial","recording_id":"01HXYZ...","text":"今天天气真"}      // ASR 增量
{"event":"segment","recording_id":"01HXYZ...","text":"今天天气真好。"}    // 已定型文本段
{"event":"audio_meter","recording_id":"01HXYZ...","meter":{"rms":0.12,"peak":0.44,"clipped":false,"vad_probability":0.82,"vad_speech":true}} // 录音中输入电平 + VAD 观测
{"event":"pipeline_step","recording_id":"01HXYZ...","name":"filler","status":"ok","duration_ms":0.3,"text":"..."}
{"event":"history","records":[...],"matched":23,"stats":{"records":23,"words":1234,"duration_ms":300000,"asr_duration_ms":260000,"asr_audio_ms":210000}} // get_history 回包，从新到旧；query 存在时带全量命中 metadata
{"event":"history_stats","snapshot":{"status":"ready","total":{"records":12,"words":345,"duration_ms":60000,"asr_duration_ms":56000,"asr_audio_ms":42000},"current_month":{...},"today":{...},"error":null}}
{"event":"history_analytics","snapshot":{"status":"ready","period":"month","anchor":"2026-06","points":[{"key":"2026-06-01","stats":{...}}],"error":null}}
{"event":"history_changed"}                                  // history JSONL 可能已变化，client 应 coalesce refresh
{"event":"audio_deleted","id":"01HXYZ...","deleted":true}
{"event":"history_deleted","id":"01HXYZ...","record_deleted":true,"audio_deleted":true,"audio_error":null}
{"event":"history_cleanup_preview","preview":{"filter":{"scope":"record_and_audio","window":{"older_than_days":30}},"ids":["01HXYZ..."],"audio_bytes":333447168,"audio_ms":5300000,"oldest":"2026-04-12T00:00:00Z","newest":"2026-06-01T00:00:00Z","warnings":[{"id":"01HBAD...","issue":"conflict"}]}}
{"event":"history_cleanup_done","result":{"requested":42,"deleted":41,"missing":1,"errors":[{"id":"01HBAD...","issue":"symlink"}]}}
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
- **`get_history` 分页**：默认 `limit=50`，最大 500，返回从新到旧。`before` 用 `started_at` RFC3339 时间戳，语义为分页游标；`before_id` 用来在同一 timestamp 下继续翻页，必须和 `before` 同时出现，单独传 `before_id` 返回 `error(kind="bad_command")`。服务端返回 `(started_at, id)` 严格早于 cursor 的记录。`query` 是可选关键词过滤，由 daemon 对 persisted JSONL records 做大小写不敏感 substring；TUI 不维护 full-history search index/fuzzy matcher。query 存在时，`history` 回包同时带 `matched` 和 `stats`，分别表示该 query 在全量 history 中的命中条数和命中集合聚合统计；分页 records 仍只返回当前页。
- **summary / analytics**：`get_history_stats` 返回全量、当前月、今天的 additive totals。`get_history_analytics` 的 `period` 为 `last_7_days|last_30_days|year|month|day`；`last_7_days` / `last_30_days` 的 `anchor` 为窗口结束日 `YYYY-MM-DD`，返回含 anchor 当天在内的最近 7/30 个按天 buckets（key 为 `MM-DD`，可跨月/年）；`year|month|day` 的 `anchor` 分别为 `YYYY`、`YYYY-MM`、`YYYY-MM-DD`，返回固定零填充 buckets：year=12 months，month=该月每天，day=24 hours。可视化平均值由 client 从 additive stats 派生。
- **stale / unavailable**：stats 和 analytics snapshot 的 `status` 为 `ready|stale|unavailable`。`stale` 表示保留 last valid index 但发现当前 JSONL 无法完全 reconcile；`unavailable` 表示没有可用 index。`error` 是给 TUI/doctor 的简短诊断，不写入 JSONL。
- **history direct response 与 broadcasts**：同一 UDS 连接上 direct command response 按该连接命令顺序返回；`history_changed` / `history_appended` 是 broadcast，和 direct response 的相对顺序不指定，client 必须 coalesce refresh。
- **删除命令**：`delete_audio` 只删 retained audio，不改 JSONL；缺失文件仍返回 `deleted=false`。`delete_history` 删除 history record 并尝试删除同 ID audio；`record_deleted=false` 表示 record 已不存在（idempotent），仍可清理 orphan audio。`audio_error` 非空表示 record 删除已完成但 audio 删除在 preflight 后失败；symlink、conflict、non-regular audio 这类危险状态在改 JSONL 前拒绝。
- **批量清理命令**：`preview_history_cleanup` / `execute_history_cleanup` 是单条 `delete_audio` / `delete_history` 的批量版本。`scope="audio_only"` 只删 retained audio，**不改 JSONL**；preview 扫描「有 retained audio 且命中 window」的 record（recording 中心，不含 orphan audio）。`scope="record_and_audio"` 删除 history record 并删除同 ID linked audio；preview 扫描所有命中 window 的 record，音频不存在仍可入选。两种 scope 都会把危险音频排除到 `warnings`（`issue` = `conflict|symlink|non_regular`）；record_and_audio 在危险音频未解决前不会删除该 record。preview 返回可安全删除的 `ids` 快照、音频总字节、语音总时长、时间范围和 warnings。**execute 只处理 preview 里回传的这批 `ids`，不按 filter 重新求值**，因此 preview 之后新增的匹配不会被删。record_and_audio 先提交 history shard，再重新核对 linked audio 的路径类型与文件 identity；音频在期间被替换时保留新文件并返回 IO error，不用隐藏 staging 文件模拟跨资源事务。`audio_only` 的 `deleted/missing` 分别表示已删/已不存在的 audio 数；`record_and_audio` 的 `deleted/missing` 分别表示已删/已不存在的 history record 数，linked audio 缺失不算 missing。危险/IO 失败进入 `errors` 且不中断整批。audio_only 不广播 `history_changed`；record_and_audio 成功删除任一 record 后广播 `history_changed`。
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
**一次有可归档内容的 recording = 一条 JSON 行**，正常完成时在 pipeline 跑完且
dispatch 完成后 append 一次；用户在 post chain 中途取消时，也只 append 一次，
记录已经完成的 pipeline steps，不写 checkpoint、不回写旧记录。启动录音前失败、
ASR 初始连接失败、麦克风 1s watchdog 判定无有效音频等
没有用户语音内容的早期失败只走 overlay / UDS error / daemon log，不写 history。同理，
**没有可归档内容的取消**（toggle 后立即 cancel，既无识别文本也没有任何带音频或
segment 的 session）也不写 history；喂过音频或已有识别文本的 cancel 仍写一条
`canceled` 记录。无 pretty print、UTF-8 无 BOM、`\n` 结尾。record JSON schema 不因分片变化升 version。

resume 热键不新增 history key，也不修改旧 history。它只读取最新一条 history：仅当最新
记录是 `canceled` 且 `asr.text` 非空，或 `timeout` + `error.kind = "asr_timeout"` 且
`asr.text` 非空时，才把旧 ASR 文本作为下一次 recording 的 seed；其他 error/timeout
和更早记录都不恢复。resume 录音的写记录判据比普通录音更严：**只有识别出新的 ASR 文本才
append**（普通录音「喂过音频或有文本」即可）——只有音频、没新文本的 resume 尝试不写记录、
不留 retained audio，否则会 append 一条空记录盖掉它想续写的那条可恢复记录、令下次 resume
断链。

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

- **顶层 `text`**：`submitted` 时是 dispatch 实际上屏的文本，显式存储，不从 pipeline 反向 derive。`canceled` 时是取消时的 ASR 文本快照；若 cancel 发生在 post chain 中途，`pipeline` 可保留已完成 step，但顶层 `text` 不改成最后一个 step 的输出。
- **`text_stats.words`**：按最终上屏文本（`text`）统计；UAX #29 word boundary，`unicode-segmentation::split_word_bounds` 过滤空白后计数。标点计入，与 Microsoft Word 中英混合"字数"同算法。
- **`asr.provider`**：已解析的后端实现种类（`"apple"` 或 `"doubao"`），来自 `AsrKind::as_str()`，是 provider 的技术标识。**注意**：这与 profile 中 `[asr] instance` 的实例 ID 是两个不同概念——实例 ID 是用户配置文件的 stem（如 `"work_doubao"`），`asr.provider` 是该实例解析后的实现种类。
- **`asr.text`**：所有 `sessions[].text` 用 provider 分隔符拼接。Continuous 模式下通常等于唯一 session 的文本；VadPause 启用时等于全部 session 的顺序拼接。resume seed 记录为 0 audio session，参与 `asr.text`，但不代表 provider 收到过旧音频。
- **`asr.duration_ms`**：ASR 工作窗口 = `sessions[-1].ended_at − sessions[0].started_at`，即 Continuous 模式会喂出去多少音频的真实基线。空 `sessions[]` 时 = 0。resume seed 的 0 audio session 使用本次第一段真实 session 的时间戳，不把误取消到继续说之间的等待时间计入 ASR 工作窗口。净节省时长按有符号数计算：`net_saved_ms = asr.duration_ms - asr.audio_ms`；正数表示跳过的静音多于 overlap 开销，负数表示 overlap 重复发送开销更大。**不要**用顶层 `duration_ms` 当分母——后者包含 post chain 等收尾时间。
- **`asr.audio_ms`**：Σ `sessions[].audio_ms`，是 provider 实际收到的音频总时长，也是按音频时长计费 provider 的计费事实。Continuous 模式下没有 resume seed 时 `audio_ms == duration_ms`（没有静音被跳过）；0 audio seed 不计入 provider 计费事实。
- **`asr.sessions[].audio_ms`**：按实际喂给 provider 的 16kHz PCM sample 数换算，
  使用 `floor(samples × 1000 / 16000)`。`started_at` / `ended_at` 也以 recording
  timeline 换算为整数毫秒，因此两种独立下取整的结果理论上最多相差 1ms。这只是
  持久化精度量化，不代表 session 在 1ms 内启停，也不影响 PCM、识别、retained
  audio 或 provider 实际收到的 sample 数。
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
- 非 resume seed session 满足 `abs(sessions[].audio_ms - (ended_at - started_at).whole_milliseconds()) <= 1`。
  两者描述同一个"首发样本→末发样本"窗口，但 sample 数和两个时间点分别量化到
  整数毫秒，允许最多 1ms 的舍入差；review 时不应将其视为生命周期或状态机缺陷。
- `asr.duration_ms == sessions[-1].ended_at − sessions[0].started_at`（空 sessions 时 = 0）。
- `net_saved_ms = asr.duration_ms as i64 − asr.audio_ms as i64`；允许为负，负值表示 resume overlap 的重复发送开销超过跳过的静音。
- Continuous 模式（当前 ASR 的 `local_vad` 解析后未启用本地 VAD）：没有 resume seed 时 `sessions.len() <= 1` 且 `asr.audio_ms == ASR 工作窗口`（没有跳过）；有 resume seed 时可多一个 0 audio seed session。
- VadPause 模式：`sessions[]` 按 `started_at` 递增；允许相邻 session 因 resume pre-roll overlap 时间重叠（`sessions[i].ended_at > sessions[i+1].started_at` 可能成立，但相邻 session 间 silence 通常远大于 overlap，所以多数情况下顺序仍递增）。
- `asr.text == sessions[].text 用 provider 分隔符拼接`
- `submitted` 且 pipeline 有 ok step 时，`text == pipeline 最后一个 ok step 的 text`；pipeline 全跳/为空时 `text == asr.text`。`canceled` 不套用这条，因为 pipeline 是观测用的已完成 steps，顶层 `text` 保留取消时的 ASR 文本。

### 2.3 启用条件

`sessions[]` 在以下条件全部成立时可包含多条：

- `~/.config/shuohua/config.toml` 中 `[voice.vad] backend = "silero"`。
- `~/.config/shuohua/asr/<id>.toml`（profile 引用的 ASR 实例文件）中 `local_vad` 解析后启用本地 VAD。

任何一个不成立时使用 Continuous 模式，`sessions[]` 最多一条。控制协议见 [modules/voice.md](modules/voice.md)。

### 2.4 空间估算

每条 ~500B 元数据 + text 长度。一天 200 次录音 ≈ 1MB。不需要压缩，后续按月 rotate 再考虑。

### 2.5 消费方契约

版本字段 `version: 1`；增字段不破坏，删字段或破坏兼容时才升 version。

History JSONL 由 `HistoryService` 通过 append/delete primitives 维护；其他模块不要直接读写 store primitives。外部脚本可以读取 JSONL，也可以在 daemon 停止时批量编辑。daemon 运行期间的外部编辑是 eventually consistent：目录 watcher 会 mark dirty，请求时 metadata fingerprint 会修复 missed event；但和任意 concurrent external writer 不提供 linearizable 语义。

读侧通过 metadata fingerprint 做稳定读取：读前 fingerprint、scan/read、读后复查 file set fingerprint，一致才接受；不一致 retry once。正常请求路径不做 content hash。文件中间损坏、完整坏行、非单调 records、symlink/non-regular shard 会让 stats/analytics 进入 stale/unavailable 或让 page 请求返回 error。

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
- **capture backend 边界**：`off` 原始采集支持 retained audio；Apple voice-processing backend
  当前不发布 retained audio。需要音频文件时，配置 `voice.preprocess.backend = "off"`。
- **一次 recording 最多一个音频文件**，含静音段。`.flac` 与 `.m4a` 不应同时存在；TUI 遇到该异常时不擅自选择。
- **收尾转换**：实时 callback 只写 `<recording_id>.tmp.wav`；录音停止并 finalize 后，用 macOS `/usr/bin/afconvert` 转成临时目标，再原子 rename 为最终文件。成功后删除临时 WAV。
- **关闭路径 (`record_audio = "off"`, 默认)** 完全跳过写入逻辑。
- **取消 (cancel) 的音频跟随「是否有内容」**：有内容的取消（喂过音频，可能是误触）
  保留 retained audio，连同 `canceled` history 记录供用户从 TUI 找回文本与音频；无内容
  的取消（toggle 后立即取消、没说话）删除临时 WAV、不生成最终文件，也不写 history，
  避免产生 TUI 无法关联的孤儿音频文件。
- **写入/转换失败**：删除所有临时音频，不保留 WAV fallback；写 daemon 诊断日志 + UDS `error(kind=audio_save)` + overlay Notice。daemon 不崩，不回滚文本 dispatch/history，下次继续尝试。
- **不存 history 字段**：路径由 ULID + 两个合法后缀约定推出，无需 `audio_path`。文件不存在 = 当时关闭、保存失败，或之后被用户删除。
- **删除语义**：TUI 的 `d` 走 `delete_audio`，只删除对应 `.flac` 或 `.m4a`，不改 history JSONL。TUI 的 `x` 走 `delete_history`，删除 history record 并删除同 ID audio。两者都 idempotent；`.flac` 与 `.m4a` 同时存在、symlink、non-regular file 等冲突/危险状态会拒绝，不跟随 symlink。
- **删除=移废纸篓（不变量）**：所有「整文件」删除（audio、以及 config 的 profile/asr/post 实例文件）都经 `crate::trash` 的 `FileDeleter` seam 移入系统废纸篓（可恢复），不永久删；**移废纸篓失败 → 记为错误并保留文件，绝不回退永久删除**。history record 是共享月度 shard 里的一行、非每条一文件，不适用，仍走 shard 重写。测试注入本地 deleter，不触碰真实 `~/.Trash`。

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
- `session_start` / `session_finalize_start` / `session_done` / `session_open_error`：
  多 session 路径下每段 ASR session 的开始、finalize 开始、结束和 resume open 失败，含 `session_index`。
- `asr_partial` / `asr_segment` / `asr_final` / `asr_error` / `asr_done`：ASR 事件时间；Doubao segment 使用服务端 `start_time/end_time`（缺失时回退到收到事件时间）。
- `recording_end`：最终 status、实际喂给 ASR 的 `audio_ms`、shadow VAD 预计
  `active_ms/saved_ms/sessions`，以及采集电平摘要 `audio_level`
  （`windows/max_rms/max_peak/clipped/has_signal`）。

trace 文件可随时删除，不被 TUI/GUI 消费。它只用于离线评估“VAD active 区间是否覆盖 ASR utterance”和“预计省费比例”。
