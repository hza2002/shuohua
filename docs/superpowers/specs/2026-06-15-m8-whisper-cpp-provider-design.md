# M8 — WhisperCppProvider 设计

> 状态：已对齐拍板（2026-06-15）。实施单元 = 一个 commit。
> 上游约束：[REQUIREMENTS.md](../../../REQUIREMENTS.md) §6 M8 行、[DESIGN.md](../../DESIGN.md) §2.8、[SCHEMA.md](../../SCHEMA.md)。
> M8 的根目的：**用一个非远端、不流式协议、错误语义完全不同的 provider 实现来验证 `AsrProvider` / `AsrSession` trait 的通用性。不动 trait。**

---

## 1. 目标与非目标

### 目标
- 接入 `whisper.cpp`（通过 `whisper-rs` 0.16），跑本地离线 ASR。
- 满足 `AsrProvider` 硬契约：流式 `Partial` + 末尾 `Segment` + `Done`。
- 配置文件与 `DESIGN.md` §2.8 的 `~/.config/shuohua/asr/whisper_cpp.toml` 占位对齐。
- 跨平台铺路：feature flag 拆 metal / cuda / vulkan，`whisper_cpp.rs` 不引 macOS-only crate。

### 非目标
- 不动 `AsrProvider` / `AsrSession` trait（动了就证伪了 trait 抽象）。
- 不引入客户端 VAD（M2.5 已决：webrtc-vad 误判高、Silero 推后）。
- 不内置模型下载（侵入性大、licensing 复杂）。doctor 提示用户去 huggingface 拉。
- 不做 idle unload 模型（复杂度高，社区 daemon 模式不做）。
- 不抽 `asr/` 成独立 crate（未来里程碑，与 M8 解耦）。
- 不扩展 doctor 到全 provider / post 检查（另起 follow-up）。
- 不动 M9 (Apple SpeechAnalyzer)。

---

## 2. 关键决策（拍板表）

| # | 决策 | 选择 | 理由 |
|---|---|---|---|
| D1 | 流式策略 | 自写 sliding-window，照 `whisper.cpp examples/stream` | rust 社区没有 production-quality streaming wrapper；自写 ~150 行可控、维护性强 |
| D2 | 配置 schema | 7 字段、命名对齐 upstream | 不发明字段名，社区惯例最强 |
| D3 | feature flag | 拆 `whisper-cpp` / `whisper-cpp-metal` / `whisper-cpp-cuda` / `whisper-cpp-vulkan`，default = metal | 为未来跨平台 asr crate 抽离铺路；M8 即刻交付 macOS metal 体验 |
| D4 | hotwords | 字符串数组 → `set_initial_prompt` 拼自然语言句 | OpenAI Cookbook 官方推荐做法 |
| D5 | doctor | 只查 metadata（toml parse + 文件存在 + size + step<length），不真加载 ctx | 加载 ~1-2s + 500MB-1GB resident，与 doctor "快、轻"原则冲突 |
| D6 | 模型生命周期 | `OnceCell<Arc<WhisperContext>>`，首次 toggle 加载，daemon 退出释放 | 与 Doubao 体验对齐（连接成本均摊到第一次）；接受常驻内存涨到 ~1GB |
| Model | 默认推荐 | `ggml-large-v3-turbo-q5_0.bin`（~574MB） | 当下 whisper.cpp 主推；中文准确率好；M1/M1 Max 都跑得动 |

---

## 3. 配置 schema

```toml
# ~/.config/shuohua/asr/whisper_cpp.toml
model          = "/path/to/ggml-large-v3-turbo-q5_0.bin"   # 必填
language       = "zh"        # ISO-639-1；中英混合场景 "zh" 比 "auto" 稳
threads        = 4           # whisper.cpp 默认值；不动态算
beam_size      = 0           # 0=Greedy{best_of=1}（快路径），>0=BeamSearch{beam_size, patience=1.0}
initial_prompt = ""          # 用户前缀；hotwords 由代码 prepend 到这之前
step_ms        = 700         # Partial 重跑步长
length_ms      = 10000       # 滚动窗口长度（whisper.cpp stream 默认 10000）
```

字段约束：
- `model`：必填非空，doctor 校验文件存在 + size > 1MB。
- `language`：whisper ISO-639-1 short code（`"zh"` / `"en"` / `"ja"` …）。空串或 `"auto"` 走自动检测。
- `threads`：clamp `1..=16`。
- `beam_size`：clamp `0..=8`。
- `step_ms`：clamp `100..=length_ms`。doctor 校验 `step_ms < length_ms`。
- `length_ms`：clamp `1000..=30000`。
- `initial_prompt`：trim；空串 == 无 prompt。

---

## 4. 数据流

### 4.1 Provider 状态

```rust
pub struct WhisperCppProvider {
    config: WhisperCppConfig,
    ctx:    OnceCell<Arc<WhisperContext>>,   // 首次 open() 时加载
}
```

- `new()`：只读 toml，不加载模型。零额外内存。
- `open()`：首次调用时加载模型（~1-2s blocking → 用 `tokio::task::spawn_blocking`），后续 open 共享 `Arc<WhisperContext>`。

### 4.2 Session 状态

```rust
pub struct WhisperCppSession {
    cmd_tx: mpsc::Sender<PcmCmd>,
    cancel: CancellationToken,
}

enum PcmCmd { Audio { samples: Vec<i16>, is_last: bool } }

// session_task 内部状态
struct SessionState {
    ctx:          Arc<WhisperContext>,
    state:        WhisperState,           // 由 ctx.create_state() 派生
    buffer_f32:   Vec<f32>,               // 累积全段 PCM，转 f32 即时转
    last_step_at: Instant,                // 上次 Partial 重跑时刻
    seq:          u64,
    cfg:          Arc<WhisperCppConfig>,
    prompt:       String,                 // hotwords prepend 后的完整 prompt，预 build
}
```

### 4.3 Session task 主循环

```text
loop {
    select! {
        biased;
        _ = cancel.cancelled() => return;
        cmd = cmd_rx.recv() => match cmd {
            None => return,
            Some(Audio { samples, is_last }) => {
                state.buffer_f32.extend(samples.iter().map(|s| *s as f32 / 32768.0));
                if is_last {
                    // 整段跑一次 final
                    let text = run_full(state, /* whole buffer */).await?;
                    evt_tx.send(Segment { text, started_at, ended_at: now }).await?;
                    evt_tx.send(Done).await?;
                    return;
                } else if state.last_step_at.elapsed() >= cfg.step_ms {
                    // 跑 sliding window
                    let window = tail(state.buffer_f32, cfg.length_ms);
                    let text = run_full(state, window).await?;
                    state.seq += 1;
                    evt_tx.send(Partial { text, seq: state.seq }).await?;
                    state.last_step_at = Instant::now();
                }
            }
        }
    }
}
```

`run_full` 在 `tokio::task::spawn_blocking` 里跑 `whisper_state.full(params, &data)`，然后通过 `state.as_iter()` 拼所有 segment text。

### 4.4 PCM 格式转换

`AsrSession::send_pcm` 收 `&[i16]`（16kHz mono）。whisper-rs 要 `&[f32]`。转换：

```rust
samples.iter().map(|s| *s as f32 / 32768.0)
```

转换发生在 session_task 内（cmd_rx 收到 audio cmd 时），不在 hot path 的 sink 端。

### 4.5 Partial 防抖

每次 sliding-window full() 都是从零重新识别（whisper 没有增量上下文），同一段音频在不同窗口下 partial 文本可能略不同。**接受这个事实**：trait 的 `Partial { text, seq }` 语义本就是"当前最新猜测，可能改写"。overlay 直接 setStringValue 替换 partial 行，没有"渐进"假设。

---

## 5. 错误映射

`WhisperError → AsrError`：

| WhisperError | AsrError | 备注 |
|---|---|---|
| `InitError` / `FailedToCreateState` | `Server("whisper init: ...")` | 模型坏 / 内存不足 |
| `FailedToEncode` / `FailedToDecode` / `UnableToCalculateSpectrogram` / `UnableToCalculateEvaluation` | `Server(...)` | 推理失败 |
| `NoSamples` | 静默 emit `Done` | 边界条件，不是错误 |
| `InvalidUtf8` / `NullByteInString` / `InvalidText` | `Protocol(...)` | 不该发生，protocol layer bug |
| `InvalidThreadCount` / `InvalidMelBands` | `Server(...)` | 配置 bug |
| `GenericError(_)` / 其他 | `Server(...)` | 底层 C++ 错误 |

**本 provider 永远不产生**：`Auth` / `Network` / `Quota` / `Timeout` / `Canceled`（`Canceled` 走 voice cancel token，不从 whisper 来）。

文件 doc-comment 顶部明说这条契约。

---

## 6. Feature flag 设计

```toml
# Cargo.toml
[features]
default = ["whisper-cpp-metal"]
whisper-cpp        = ["dep:whisper-rs"]
whisper-cpp-metal  = ["whisper-cpp", "whisper-rs/metal"]
whisper-cpp-cuda   = ["whisper-cpp", "whisper-rs/cuda"]
whisper-cpp-vulkan = ["whisper-cpp", "whisper-rs/vulkan"]

[dependencies]
whisper-rs = { version = "0.16", optional = true, default-features = false }
```

`whisper_cpp.rs` 整文件包在 `#[cfg(feature = "whisper-cpp")]`。

`main.rs::build_provider`：

```rust
match name {
    "doubao" => ...,
    #[cfg(feature = "whisper-cpp")]
    "whisper_cpp" => Ok(Arc::new(whisper_cpp::WhisperCppProvider::new()?)),
    #[cfg(not(feature = "whisper-cpp"))]
    "whisper_cpp" => anyhow::bail!(
        "binary 未启用 whisper-cpp feature；重新编译 `cargo build --features whisper-cpp-metal` 或选其他 provider"
    ),
    other => anyhow::bail!("未知 ASR provider {other:?}"),
}
```

`Cargo build --no-default-features` 时 whisper_cpp 分支编译为友好 error 提示，二进制仍可用其他 provider。

---

## 7. 模块拆分

```
src/asr/providers/whisper_cpp.rs   ← 新增整个模块（feature-gated）
```

文件 ~250 行（含 doc + 单测）：

```text
// 0. Module doc：契约、错误映射、不产生哪些 AsrError 变体、跨平台说明
// 1. WhisperCppConfig + 默认值 + load_config
// 2. WhisperCppProvider + new + Arc<WhisperContext> 加载
// 3. WhisperCppSession + send_pcm/close
// 4. session_task：sliding-window loop
// 5. run_full：spawn_blocking 包装 whisper-rs full()
// 6. map_err：WhisperError → AsrError
// 7. tests：config parse / clamp / hotwords prompt 拼接 / map_err
```

`src/asr/providers/mod.rs` 加 `#[cfg(feature = "whisper-cpp")] pub mod whisper_cpp;`。

---

## 8. 测试策略

**纯函数单测**（feature 开时编进 binary）：

| 测试 | 验证 |
|---|---|
| `config_parse_minimal` | 只有 `model` 字段时其他字段默认值正确 |
| `config_parse_full` | 全字段 round-trip |
| `config_clamp_thread_count` | threads=99 clamp 到 16 |
| `config_validate_step_lt_length` | step_ms >= length_ms 时 doctor 校验返回 hint |
| `build_initial_prompt_with_hotwords` | hotwords + initial_prompt 拼接顺序正确 |
| `build_initial_prompt_empty_hotwords` | 空 hotwords 时不加 prefix |
| `build_initial_prompt_truncates_long` | hotwords 过长时截断 + 不破坏 UTF-8 边界 |
| `map_err_init_to_server` | `WhisperError::InitError` → `AsrError::Server` |
| `map_err_invalid_utf8_to_protocol` | `InvalidUtf8` → `AsrError::Protocol` |
| `pcm_i16_to_f32_normalization` | i16::MIN/MAX → f32 -1.0/~1.0 |

**集成测试**：不引入。真实 whisper context 加载要模型文件，CI 跑不动；用户手工验收即足够 trait 通用性证明（验收标准本身）。

**fake provider 路径不变**：M2 已有的 voice 状态机 + FakeProvider 单测继续跑，验证我们没动 trait。

---

## 9. main.rs / doctor.rs 改动

### main.rs::build_provider

按 §6 加 `#[cfg(...)]` 分支。约 5 行新增。

### cli/doctor.rs::check_asr_provider

匹配 `"whisper_cpp"` 时：
1. parse `~/.config/shuohua/asr/whisper_cpp.toml`
2. 检查 `model` 文件存在 + 文件大小 > 1MB
3. 校验 `step_ms < length_ms`
4. feature 关闭时打 `WARN binary built without whisper-cpp feature`
5. **不**加载 ctx

输出格式跟 doubao 分支对齐：

```text
asr.whisper_cpp: OK config readable (profile="Default", model=ggml-large-v3-turbo-q5_0.bin (574MB), language="zh", threads=4)
asr.whisper_cpp: no inference run; model not loaded
```

---

## 10. 文档更新

| 文件 | 改动 |
|---|---|
| `REQUIREMENTS.md` 决策表 | 加一行：M8 whisper_cpp 接入决策摘要 |
| `docs/DESIGN.md` §2.8 表 | 把 "whisper.cpp 流式包装" 行补全描述（实际策略 = sliding-window）|
| `docs/DESIGN.md` §2.8 配置文件布局示例 | 把 `whisper_cpp.toml` 示例从"未来 M8"改为现状 |
| `docs/DESIGN.md` §7 安全与隐私 | 加一条：whisper_cpp 启用时 daemon 常驻 ~1GB |
| `docs/MODULES.md` | 把 `asr/providers/whisper_cpp.rs` 从"未实现"挪到"已实现 M8"段；详细职责一句话 |
| `Cargo.toml` | 加 `[features]` + `whisper-rs` optional dep |

---

## 11. 验收清单

- [ ] `cargo fmt` 干净
- [ ] `cargo check` 默认 feature OK（含 metal 编译 whisper.cpp 静态库；耗时可接受）
- [ ] `cargo check --no-default-features` OK（whisper_cpp 模块整体编译被剔除）
- [ ] `cargo test` 全过（含 §8 单测；fake provider voice 状态机测试照旧）
- [ ] 用户手工：下载 `ggml-large-v3-turbo-q5_0.bin` → 改 `apps/default.toml` 切到 `provider = "whisper_cpp"` → F16 录一段中英混合 → overlay 流式 Partial 可见 → 停后 1-2s 出 Segment → 剪贴板 + 自动粘贴
- [ ] `shuo doctor` 在配置切到 whisper_cpp 时打印正确状态

---

## 12. Follow-ups（不在 M8 范围）

- doctor 遍历全 profile + 校验所有 post 组件（rules/llm 能否 parse + LLM endpoint 可达）。独立小里程碑。
- `asr/` 抽成独立 `shuohua-asr` crate，真正解锁跨平台复用。需要先拆 trait + types 跟 history/state/overlay 解耦。
- Whisper VAD 接入（whisper-rs 0.16+ 支持 Silero ggml VAD 模型），让 partial 之间能切句末 Segment，体验向 Doubao 靠拢。需要先下 vad model。
- idle unload 模型（如果用户反映常驻内存 1GB 太重）。
