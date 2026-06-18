# M10 Multi-session ASR Research

日期：2026-06-16
状态：前期调研；不是实现 spec，不改 schema

## 背景

M9 已把 history schema 升到 v2，并预留 `asr.sessions[]`。当前阶段限制仍是 `sessions.len() == 1`；M10 只应解除写入侧的单 session 假设，不应再改 schema。

当前代码路径：

- `src/voice/recorder.rs`：cpal callback 把默认输入设备转成 16kHz mono s16le `Vec<i16>`，通过 mpsc 给 voice；可选同源 WAV 留存。
- `src/voice/finish.rs`：一次 recording 打开一次 `provider.open()`，每帧 PCM 都 `send_pcm(false)`；stop 时 drain `stop_delay_ms`，发 `send_pcm([], true)`，等 `Done`。`SegmentCapture` 现在累积 provider 的定型文本段，history 写入时折叠成一条 `AsrSessionHistory`。
- `src/asr/providers/doubao.rs`：每个 session 是一条 Doubao WS 连接 + init frame + audio frames；`close()` 取消 task 并关 WS。
- `src/asr/providers/apple.rs`：每个 session spawn 一个 embedded Swift helper 子进程；helper 内跑 `SpeechAnalyzer + SpeechTranscriber`，stdin 收 PCM，stdout 发 JSON event。

## A. 本地 VAD 选型

### 评价维度

评分 1-5，5 最好。分数用于暴露 tradeoff，不做机械排序；VAD 默认选择仍以真实 fixture bakeoff 为准。

| 维度 | 含义 |
|---|---|
| 依赖成本 | 新增 crate / C 编译 / 动态库 / 模型文件 / 打包复杂度 |
| 误判风险 | 对呼吸、键盘、风扇、远场噪声的主观稳健性预期 |
| CPU / 内存 | 是否适合常驻 daemon 实时跑 |
| 可测试性 | 是否能用固定 PCM fixture 做纯函数/近纯函数单测 |
| 实现复杂度 | 接入 recorder/voice 主循环的代码量和调参面 |

### 方案 1：`webrtc-vad` crate

资料：

- `webrtc-vad 0.4.0` 是 libfvad / WebRTC VAD 的 Rust binding，build dependency 只有 `cc`，需要本机 C compiler。
- docs.rs 显示输入采样率支持 8/16/32/48kHz；frame 只能是 10/20/30ms；mode 0-3，越 aggressive 越少报 speech，但 missed detection 也会上升。
- `Vad` 自动 trait 是 `!Send` / `!Sync`，应把实例限制在单 task/单 controller 内。

评分：

| 维度 | 分数 | 说明 |
|---|---:|---|
| 依赖成本 | 5 | 小 C 库，编译成本低，无模型文件，无运行时 dylib |
| 误判风险 | 2 | 真实项目历史已踩过：风扇/空调/键盘类噪声容易把“非语音”判成“有声”；WebRTC VAD 本质偏实时通信老模型 |
| CPU / 内存 | 5 | 10-30ms 帧级 GMM/fixed-point，开销可忽略 |
| 可测试性 | 4 | 固定 PCM frame 可测；但 `Vad` 内部有状态，测试要显式 reset |
| 实现复杂度 | 4 | 16kHz PCM 已满足；只要做 10/20/30ms framing + 滑窗/hysteresis |
| 总分 | 20/25 | 适合作 baseline，不建议直接作为生产默认 |

结论：不作为 M10 默认 VAD。可以作为 bakeoff 对照组，或者作为“拒绝模型依赖时的低成本 fallback”。

### 方案 2：Silero VAD via ONNX Runtime (`ort` / `voice_activity_detector`)

资料：

- Silero 官方 README：ONNX 路径依赖 `onnxruntime`；16kHz/8kHz；JIT model 约 2MB；30ms+ chunk 单 CPU thread 通常小于 1ms；模型覆盖多语言、多背景噪声。
- PyTorch Hub 页面明确说模型面向 CPU，按 1 个 CPU thread 优化，量化模型。
- `voice_activity_detector 0.2.1` 是 Rust Silero VAD V5 wrapper，依赖 `ort 2.0.0-rc.10` / `ort-sys` / `ndarray`；docs.rs 写明 16kHz 固定 512 samples、8kHz 固定 256 samples，不合长度会 pad/truncate。
- `ort 2.0.0-rc.12` 默认 features 包含 `download-binaries`、`copy-dylibs`、`api-24`；Context7 文档显示也可用 `load-dynamic` 手动加载 `libonnxruntime.dylib`。

评分：

| 维度 | 分数 | 说明 |
|---|---:|---|
| 依赖成本 | 2 | ONNX Runtime dylib/下载/打包是主要成本；模型文件也要进 artifact 或 cache；精确体量需本机 release 包测量 |
| 误判风险 | 4 | 主观预期明显优于 RMS/WebRTC；仍需用用户呼吸、键盘、风扇 fixture 验证 false positive |
| CPU / 内存 | 4 | 512 samples = 32ms/frame；CPU 应可忽略，但 ORT runtime 常驻内存需测 |
| 可测试性 | 4 | 固定 PCM → probability，可做 deterministic fixture；初始化 ORT/模型让单测偏集成，核心 hysteresis 仍可纯函数测 |
| 实现复杂度 | 3 | 需要模型资产、ORT 初始化、概率阈值、状态保持、错误降级 |
| 总分 | 17/25 | 最有希望成为 M10 默认，但必须先做本机 bakeoff |

结论：M10 首选候选。推荐路径是优先评估 `voice_activity_detector`，因为它已经封装 Silero V5 + streaming helper；如果 crate 行为或维护质量不合适，再退到直接用 `ort` 跑 Silero ONNX。

### 方案 3：Silero VAD via Candle

资料：

- Candle 是 Hugging Face 的 Rust ML framework；Context7 文档显示它擅长 CPU tensor、safetensors 权重、Rust-native model forward。
- Candle 不是 ONNX Runtime 的 drop-in replacement。要跑 Silero，通常意味着找/写 Candle 版模型结构和权重加载，或维护转换产物。

评分：

| 维度 | 分数 | 说明 |
|---|---:|---|
| 依赖成本 | 3 | 避免 ORT dylib，但引入 ML framework；若启用 Metal/Accelerate 又增加平台分支 |
| 误判风险 | 4 | 模型本身可接近 Silero，前提是实现等价 |
| CPU / 内存 | 4 | CPU tensor 小模型可接受 |
| 可测试性 | 4 | 固定模型 + fixture 可测 |
| 实现复杂度 | 1 | 需要确认/维护 Silero 架构实现，M10 过重 |
| 总分 | 16/25 | 不适合作第一版 |

结论：暂不选。除非后续 ORT 打包成本不可接受，且已有成熟 Candle Silero 实现可复用。

### 方案 4：Apple SpeechDetector / SpeechAnalyzer

资料：

- Apple WWDC25 SpeechAnalyzer 资料：`SpeechAnalyzer` 管理 analysis session，module 处理不同分析；audio input 和 results 都是 async sequence；SpeechTranscriber 是 on-device，模型不增加 app binary size。
- 本机 macOS 26 SDK `Speech.swiftinterface` 确认存在 `SpeechDetector`：`init(detectionOptions:reportResults:)`、`SensitivityLevel.low/medium/high`、`results` async sequence；`Result` 包含 `range`, `resultsFinalizationTime`, `speechDetected: Bool`。
- 当前 Apple helper 只用了 `SpeechTranscriber`，没有挂 `SpeechDetector`。

评分：

| 维度 | 分数 | 说明 |
|---|---:|---|
| 依赖成本 | 4 | 无第三方依赖；但只在 macOS 26+，且要改 Swift helper |
| 误判风险 | 4 | Apple 本地模型预期强；真实延迟和 false positive 需实测 |
| CPU / 内存 | 4 | 系统模型；但与 transcriber/helper 生命周期耦合 |
| 可测试性 | 2 | 依赖 macOS Speech framework 和资产，纯单测困难；只能小范围集成测 |
| 实现复杂度 | 2 | Rust voice 层要消费 helper detector event；若拿它控制 Doubao，会出现“为了省 Doubao 费先跑 Apple Speech”的产品语义混乱 |
| 总分 | 16/25 | 可研究，但不适合 M10 第一版控制 Doubao |

结论：不作为 Doubao idle pause 的默认 VAD。可以后续用于 Apple provider 自身的 endpoint/UX 优化，但 Apple provider 不计费，M10 不应为它引入多 session。

### 方案 5：升级版 RMS（滑窗 + 多频带能量 + 滞回）

资料/现状：

- 当前代码已有 `frame_has_signal()`，只用于“1s 内是否看到非零麦克风信号”的设备 watchdog；阈值极低，不是 VAD。
- 旧版朴素 RMS + 阈值已在呼吸/键盘/风扇场景不稳。

评分：

| 维度 | 分数 | 说明 |
|---|---:|---|
| 依赖成本 | 5 | 零依赖 |
| 误判风险 | 2 | 多频带 + hysteresis 能减少抖动，但仍无法可靠区分人声和稳定噪声/冲击声 |
| CPU / 内存 | 5 | 可忽略 |
| 可测试性 | 5 | 完全纯函数，可大量 fixture/proptest |
| 实现复杂度 | 3 | DSP 逻辑不大，但调参会持续消耗时间 |
| 总分 | 20/25 | 适合作辅助特征，不适合作最终判定 |

结论：不建议再把 RMS 打磨成主 VAD。可以作为前置 gate：低能量直接判 silence，高能量再交给 Silero，减少模型调用和处理合盖/静音 buffer。

### A 推荐

推荐 M10 的 VAD 路线：

1. 做离线 bakeoff：采集/复用本机 fixture（正常说话、轻声、呼吸、机械键盘、风扇、空调、远场背景、静默），同一份 16k PCM 跑 WebRTC / Silero / RMS baseline。
2. 若 Silero 在 false positive 上明显优于 WebRTC/RMS，则 M10 默认选 Silero via `voice_activity_detector` 或直接 `ort`。
3. WebRTC 只保留为低成本 fallback / 对照，不作为默认。
4. Apple `SpeechDetector` 不进入 Doubao 多 session 控制路径；另开后续调研 Apple provider endpoint。

开放问题（必须实测）：

- `voice_activity_detector` 在 Apple Silicon 上的 release binary 增量、首次初始化耗时、常驻 RSS。
- ORT 默认 `download-binaries/copy-dylibs` 是否符合 shuohua 发布方式；是否改用 `load-dynamic` 并随 app 分发 dylib。
- Silero threshold / min speech frames / min silence duration 在用户真实环境下的 false positive/false negative。
- 呼吸、键盘、风扇 fixture 下 Silero 是否仍会把非语音判成 speech。
- VAD 放在 tokio voice task 内是否足够，还是需要独立轻量 task 防止 ASR send 背压影响判定时间。

## B. 多 session 控制协议设计

### 评价维度

评分 1-5，5 最好。

| 维度 | 含义 |
|---|---|
| 成本收益 | 对 Doubao 计费时长的节省是否明确 |
| 延迟风险 | resume 后用户感知延迟、首字丢失、文本重复风险 |
| 简单性 | 对现有 `finish.rs` / provider trait / recorder 的侵入程度 |
| 可测试性 | 是否能用 fake provider + PCM fixture 覆盖状态转移 |
| 用户可理解性 | overlay/TUI 行为是否符合“录音仍在进行”的心智 |

### 目标流程

给定一个本地 VAD 信号源：

1. recording 开始时麦克风一直开，voice 层持续收 16k PCM。
2. `Recording.Active`：VAD 判 speech 时把 PCM 喂当前 ASR session；VAD 判 silence 达阈值后，对当前 ASR session 发 `is_last=true` 并等 `Done`，把该 session 的文本和 `audio_ms` 固化到内存。
3. `Recording.Idle`：麦克风继续开，VAD 继续跑；不持有 Doubao WS，不向 ASR 计费。
4. VAD 判 speech 恢复时，voice 层开新 ASR session，并 replay pre-roll PCM，再喂当前和后续 PCM。
5. 用户 toggle OFF 时，不管 Active/Idle，都停止麦克风，跑 post chain，append 一条 history；`asr.sessions[]` 写多条。

### 方案评价

#### 1. pause 阈值

候选：

| 阈值 | 评价 |
|---|---|
| 800ms | 省费更激进，但中文口述里的自然停顿、思考停顿容易切得太碎 |
| 1500ms | 比较平衡；短句停顿不立刻切，思考停顿能停止计费 |
| 3000ms | M2.9 旧设计偏保守，省费变弱；“想几秒再说”才触发 |

推荐：先用 `pause_asr_silence_ms = 1500` 做实验默认，但配 hysteresis：

- VAD speech probability 连续低于 silence threshold 达 1500ms 才 pause。
- resume 至少要求 2 个 voiced frames 或 64-100ms 内累计 voiced，以免键盘单击触发重连。
- stop 阶段仍保留现有 `stop_delay_ms`，不要和 pause 阈值混用。

还需实测：Doubao 末段 final 延迟 + 1500ms silence 是否会让用户感觉尾字慢；长句中 1-2 秒自然停顿是否被切得过碎。

#### 2. resume 是否 replay 300ms PCM

候选：

| 方案 | 评价 |
|---|---|
| 不 replay | 最简单，但 VAD 判定需要若干帧，容易吃掉开头辅音/轻声 |
| replay 150ms | 延迟小，但对 Silero 512-sample frame + resume hysteresis 可能不够 |
| replay 300ms | 常见 pre-roll，能覆盖 VAD 判定窗口和用户起声前缘，重复风险可控 |
| replay 500ms+ | 更稳但增加重复/多喂音频，省费收益降低 |

推荐：resume 时 replay 最近 300ms PCM。

buffer 位置推荐放 voice 层，不放 recorder 端：

- recorder 继续只负责 canonical PCM producer + optional WAV，保持简单。
- voice 层需要同时做 VAD、session 状态、audio_ms 计数；pre-roll 是否计入新 session audio_ms 也应在这里统一决定。
- 纯函数测试可以喂 PCM frame 序列，断言状态转移、replay 样本数、session audio_ms。

还需实测：300ms 是否导致 Doubao 新 session 开头重复上一个 session 的尾字；必要时只在 Idle→Active replay，不在初始 Active replay。

#### 3. Doubao WS 重连握手延迟

本轮已跑 `/tmp/shuohua-doubao-handshake-probe`，只测 WS upgrade + init frame，不发送真实语音：

| 次数 | connect | init send | first server response | total |
|---:|---:|---:|---:|---:|
| 0 | 190ms | 0ms | 66ms | 258ms |
| 1 | 195ms | 0ms | 61ms | 257ms |
| 2 | 180ms | 0ms | 68ms | 249ms |
| 3 | 206ms | 0ms | 60ms | 267ms |
| 4 | 198ms | 0ms | 68ms | 267ms |

结论：当前网络下，Doubao session reopen 的纯握手成本约 250-270ms。加上 VAD resume hysteresis 和首个 partial，用户可感知恢复延迟可能接近 400-700ms。

还需实测：发送 300ms pre-roll + 真实语音后，Idle→Active 到首个 partial / first Segment 的端到端延迟；不同网络/VPN 状态下 p95。

#### 4. provider trait：`supports_idle_pause()` 还是配置

候选：

| 方案 | 评价 |
|---|---|
| provider caps 显式能力 | 行为贴 provider 实现；doctor/TUI 可展示；避免用户把 Apple 配成“省费多 session” |
| 配置项控制 | 灵活，但把 provider 语义推给用户，容易产生无收益或高 churn 配置 |

推荐：走 trait/caps，不走用户配置。M10 spec 阶段可考虑在 `Caps` 增加 `idle_pause: bool` 或 `supports_idle_pause()`。

初始能力建议：

- Doubao：true。按音频时长计费，关 session 有明确收益。
- Apple：false。on-device 不计费，且每 session spawn helper，频繁切换只有 churn，没有直接收益。

#### 5. 重新 `provider.open()` 失败策略

候选：

| 策略 | 评价 |
|---|---|
| 立刻终止整次 recording | 简单，但一次网络抖动会丢掉用户后续讲话 |
| 保持 recording，进入 IdleError，下一次 voiced 再重试 | 用户可继续说；但要避免无限重试和 UI 噪声 |
| 本段本地缓存 PCM，等 open 成功后补发 | 可靠但复杂，静音省费目标会变成本地队列/重放系统 |

推荐：保持 recording，不缓存长音频。

具体行为：

- Idle→Active open 失败：发一次 overlay notice（非 fatal），记录 pending error，留在 Idle，继续跑 VAD。
- 对同一段 voiced burst 不忙等重试；至少等回到 silence 再允许下一次 open，或加指数退避（如 1s/2s/5s cap）。
- 如果整次 recording 至少已有一个成功 session，用户 stop 时用已有文本继续 post/dispatch；history 可保留 error note 需要后续 schema 扩展时再讨论，本轮不改 schema。
- 如果没有任何成功 session，stop 后按当前错误/取消语义处理，不上屏空文本。

还需拍板：history v2 当前没有 session-level error 字段；M10 不改 schema 的前提下，open 失败只能进 stderr/overlay/UDS transient，不能持久化到 `asr.sessions[]`。

#### 6. overlay idle 反馈

候选：

| 方案 | 评价 |
|---|---|
| idle 期间隐藏/透明 | 干扰最小，但用户可能以为录音已停；也看不到“省费中/继续听着” |
| 保持 panel，状态改 Listening/Paused | 语义清楚；轻微占屏 |
| 只保留状态点/小尺寸 panel | 平衡，但需要额外 UI 状态设计 |

推荐：M10 初版保留 overlay，可弱化但不要完全透明。建议状态文案为 `Listening` 或 `Paused`，红点切灰/蓝，文本区保留已识别 segments，不显示“使用说明”。理由：用户按 F16 后 recording 仍在进行，完全隐藏会破坏可见状态，也让 open 失败 notice 没地方落。

还需实测：长时间思考时 overlay 是否遮挡工作流；可在后续 UX 调整中加“idle N 秒后收缩”。

#### 7. Apple provider 是否支持多 session

推荐：M10 不支持。

理由：

- Apple on-device 不按时长计费，M10 的主要收益不存在。
- 当前每 session spawn Swift helper；多 session 会增加子进程 churn、资产/model retention 不确定性和结果拼接复杂度。
- Apple Speech framework 已有自己的 endpoint/finalization 行为；如果要利用 `SpeechDetector`，应作为 Apple provider 内部优化或单独 milestone，不应和 Doubao 省费机制耦合。

### B 方案评分

| 方案 | 成本收益 | 延迟风险 | 简单性 | 可测试性 | 用户可理解性 | 总分 | 结论 |
|---|---:|---:|---:|---:|---:|---:|---|
| 关 session：silence 后 close，speech 后 reopen + replay | 5 | 3 | 4 | 4 | 4 | 20/25 | M10 推荐 |
| 保持 WS：silence 后暂停喂 PCM，speech 后继续喂 | 4 | 4 | 2 | 3 | 3 | 16/25 | 暂不选；需心跳/idle timeout 实测 |
| 只做本地 VAD，不关 ASR session | 1 | 5 | 5 | 5 | 4 | 20/25 | 不满足省费目标 |
| Apple provider 也多 session | 1 | 2 | 2 | 2 | 2 | 9/25 | 不选 |

说明：第三项总分高是因为它简单低风险，但不解决 M10 的核心目标（Doubao 静音不计费），所以不推荐。

### B 推荐

推荐的 M10 控制协议方向：

- 多 session 只对 provider caps 声明支持的 provider 开启；初始只有 Doubao。
- recorder 保持不变；voice 层维护 VAD、pre-roll ring buffer、Active/Idle 状态、session records。
- pause 默认 1500ms silence + hysteresis；resume replay 300ms PCM。
- Doubao 使用“关 session”方案，而不是“保持 WS 暂停喂音频”。原因是当前探针显示 reopen 纯握手约 250-270ms，可接受；保持 WS 需要心跳/服务端 idle timeout 语义，复杂度更高。
- open 失败不立刻杀整次 recording；留在 Idle 并给 notice，下个 voiced burst 再重试。
- overlay idle 期间保留弱反馈，不完全隐藏。
- Apple provider M10 不参与多 session。

开放问题（必须实测/拍板）：

- Silero/WebRTC/RMS bakeoff 的真实 false positive/false negative，尤其是呼吸、键盘、风扇。
- Doubao Idle→Active 端到端延迟：VAD resume 判定 + open + replay + first partial / first segment。
- 1500ms pause 是否切断用户自然表达；是否需要 per-user 配置。
- 300ms replay 是否造成新旧 session 文本重复。
- open 失败 transient 信息是否需要未来 schema 扩展；M10 不改 schema 时只能不持久化。
- overlay idle 反馈文案和视觉状态是否需要 i18n 新 key；这属于后续 spec，不在本调研落实现。

## Sources

- Project docs/source: `CLAUDE.md`, `docs/DESIGN.md`, `docs/SCHEMA.md`, `docs/MODULES.md`, `CHANGELOG.md`, `src/voice/finish.rs`, `src/voice/recorder.rs`, `src/asr/providers/doubao.rs`, `src/asr/providers/apple.rs`, `src/asr/providers/apple_helper.swift`.
- Context7: `/pykeio/ort` setup/linking and session docs; `/huggingface/candle` tensor/model loading docs.
- `webrtc-vad` docs.rs: https://docs.rs/webrtc-vad/latest/webrtc_vad/struct.Vad.html
- `voice_activity_detector` docs.rs: https://docs.rs/voice_activity_detector/latest/voice_activity_detector/
- Silero VAD official repo: https://github.com/snakers4/silero-vad
- PyTorch Hub Silero VAD page: https://pytorch.org/hub/snakers4_silero-vad_vad/
- Apple WWDC25 SpeechAnalyzer transcript: https://developer.apple.com/videos/play/wwdc2025/277/
- Local SDK interface checked at: `/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk/System/Library/Frameworks/Speech.framework/Versions/A/Modules/Speech.swiftmodule/arm64e-apple-macos.swiftinterface`
