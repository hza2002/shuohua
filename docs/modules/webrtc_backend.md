# WebRTC APM backend 参数设计

> 状态：`feat/webrtc-apm-backend`。POC 之上定档：APM 固定 16k、锁定一套 curated
> preset、用户面只保留 `backend` 一个参数。本文记录**为什么这么定**——参数取舍、
> 刻意不接的能力和其理由，这些从代码读不出来。
>
> **代码**：`src/voice/webrtc_apm.rs`（`curated_config()` 是唯一的参数出处）。
> **不变量/边界**：见 [voice.md](voice.md)（"WebRTC backend 是……保持接近 off 的启动路径"）。
> **用户配置**：见 [config.md](config.md)（`[voice.preprocess].backend`）。

## 定位

三种采集路径，`[voice.preprocess].backend` 选择：

- `webrtc`（默认）：cpal 采集 + 进程内 WebRTC Audio Processing。跨平台、纯软件的降噪+
  增益清理；不需要系统权限、不 ducking，启动路径接近 `off`。`webrtc-audio-processing`
  是普通依赖（`features = ["bundled"]`），随每次构建编入 bundled C++——不是 feature、关不掉。
- `off`：cpal 原始采集，不预处理。安静环境识别也好、开头不丢字，且是唯一发布**完整原始**
  retained audio 的路径（webrtc/apple 留存的是处理后音频）。
- `apple`：macOS 系统语音处理（`setVoiceProcessingEnabled`），自带 AEC/降噪/AGC，
  **回声/外放场景（一边播放一边说话）用它**。定位是音频环境差时的兜底。见
  [apple_backend.md](apple_backend.md)。

`webrtc` 与 `apple` 的分工：`apple` 靠 OS，有 AEC 但会短暂 ducking、和 cpal 共享同
一物理设备、有 VP 冷启动与 wedge 风险；`webrtc` 是自包含的用户态处理，无 AEC 但启动
路径接近 `off`、无平台副作用。

## 用户面：只有 `backend` 一个参数

**刻意不暴露任何 WebRTC 子参数**（NS 档位、AGC mode/target、HPF 开关、pre-gain、
strength preset 都不给）。理由：这些旋钮要求用户理解 dBFS / AGC / 降噪失真等音频概念，
且绝大多数取值组合只会让效果更差；"默认就够好"比"可调"更重要。想要不同清理力度的
用户在 `off` / `webrtc` / `apple` 之间选即可。

若将来确有需求，唯一可考虑的增量是一个粗档 `strength = light|balanced|strong`（映射到
内部 preset），**但不到有实测证据前不做**，更不暴露原始参数。

## curated preset（`curated_config()`）

固定启用，全部是 WebRTC 稳定主干能力：

| 子模块 | 取值 | 为什么 |
|---|---|---|
| high-pass filter | default（full-band） | 去低频隆隆声，语音必备；NS 本就强制启用它，显式写只为表明意图 |
| noise suppression | `Moderate` | 见下方"NS 档位" |
| gain controller | AGC1 `AdaptiveDigital`，target=6，comp=9，limiter=on | 见下方"AGC" |

### APM 固定 16 kHz

APM 在 16k 上跑，不跟随设备率。16k 是 ASR 的 canonical 目标率，也是 WebRTC 最成熟的
工作率（覆盖 0–8kHz，正好是 ASR 关心的全部频带）。设备任意率 →（一次）重采样到 16k
→ APM@16k → recorder 的 `Resampler16k` 退化成 passthrough。**全链路只 1 次重采样**；
旧实现只认 16/32/48k、其它先拉到 48k 再降到 16k，会重采样两次。

### NS 档位：`Moderate`（= crate 默认）

- 四档 Low/Moderate/High/VeryHigh。对**ASR 前处理**，默认取 `Moderate`：宁可欠抑制也
  别过抑制——过度 NS 会连语音谐波/清辅音一起削掉、升 WER，而现代 ASR 声学模型本身对
  噪声有鲁棒性，不需要前面猛清理。`Moderate` 也是 crate 作者 tune 的平衡档。
- **举证责任在"升档"一侧**：更激进的 `High`/`VeryHigh` 只有实测证明对识别有**净增益**
  才升，不因为"清理多点是白赚"就默认拉高。
- 改档只改 `curated_config()` 里一处，单测 `curated_config_locks_voice_tuned_preset`
  会锁值。**验证要看识别文本本身**（同句子在 off / Moderate / High 下对比掉字/错字），
  不要靠电平数字——RMS/dBFS 只反映"抑制了多少"，反映不了"有没有把语音也削了"。

### AGC：AGC1 `AdaptiveDigital`（不是 crate 默认）

- **必须是 `AdaptiveDigital`，不能用 crate 默认的 `AdaptiveAnalog`**：analog 模式要求
  通过 `stream_analog_level()` 把 OS 麦克风音量耦合进 AGC 形成闭环；我们没接线，analog
  段会空转。单测专门锁死 mode 防退回默认。
- `target_level_dbfs=6`（即 -6 dBFS）比 crate 默认 -3 dBFS 多留 3dB headroom，降低顶到
  limiter/削波的概率，对 ASR 友好。
- `compression_gain_db=9` + `limiter=on`：温和拉抬 + 防过冲。这套即针对人声的安全默认。

## 刻意不接的能力

| 能力 | 决定 | 理由 |
|---|---|---|
| **AEC**（echo canceller） | 不接 | 需要"系统正在播放什么"的 render 参考帧（`process_render_frame` + 延迟对齐），我们没有 system-audio loopback。回声/外放场景用 `backend = "apple"`（OS 自带 AEC）。WebRTC AEC 是一整个采集特性、不是配参数，且我们不主动维护跨平台 AEC。无限期押后。 |
| **AGC2** | 不接 | 卖点 input volume controller 要读写 CoreAudio HAL 麦克风音量（`stream_analog_level`），我们接不上、crate 也没干净暴露；剩下的 adaptive-digital 与 AGC1 `AdaptiveDigital` 功能重叠。换过去零收益、纯回归风险。 |
| **capture amplifier / pre-gain** | 不接（保持 `None`） | 固定前置增益对大声输入会削波，而 AGC `AdaptiveDigital` 已自适应把音量拉到目标，严格优于拍脑袋的固定值。真遇到"连 AGC 都拉不动的极端小声麦"再内部微调，也不暴露。（注：legacy `PreAmplifier` 上游已标 deprecated，要做用 `CaptureLevelAdjustment`。） |
| **stats**（`get_stats`） | 不接 | `Stats` 5 个字段全是 AEC 相关（ERL/ERLE/residual-echo/delay），不开 echo_canceller 时全返回 `None`。**没有 NS/AGC 指标**。要调参遥测就自己在 dev feature 下算 pre/post RMS dBFS，别记一堆 None。 |
| **pipeline**（多通道/降混/内部率） | 不接 | mono 场景默认即最优。 |
| **runtime hints**（`set_output_will_be_muted` / `set_stream_key_pressed` / `reinitialize`） | 不接 | 前两个主要利好 AEC/AGC 适配，无 AEC 时收益极小；`reinitialize` 用不上——`PcmProcessor` 每次录音新建 `Processor`，天然是干净状态，无跨录音污染。 |
| **experimental-\* feature**（aec3-config / unlink-ns） | 不接 | crate 明示 experimental，只服务 AEC/多通道 corner case。 |

## 集成边界

- APM 只在 recorder 线程内、`PreprocessStage::WebRtc` 分支起作用；10ms frame chunking
  由 `FrameChunker` 保证（WebRTC `process_capture_frame` 帧长不符会 panic）。engine 不
  感知 WebRTC 细节，backend 对外契约仍是 16k mono i16、可 drain residual。见 voice.md。
- `curated_config()` 是唯一的参数出处，配套单测 `curated_config_locks_voice_tuned_preset`
  锁死关键值（尤其 AGC mode），防止跟随 crate 默认悄悄漂移。

## 仍需人工验证

- [ ] NS 档位：默认 `Moderate`。挑有清辅音/轻声尾字的句子（"是不是""谢谢""四十四"）
      在 off / `Moderate` / `High` 下各说几遍，**对比 ASR 出的文本**是否掉字/错字；只有
      `High` 证明有净增益才考虑升档。
- [ ] `webrtc` vs `off` 在安静环境：确认开头不丢字、增益不过冲。
- [ ] `webrtc` vs `apple` 在嘈杂/外放环境：确认各自定位（webrtc 无 AEC 的边界）。
