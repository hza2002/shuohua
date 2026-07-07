# Apple voice processing backend

> `voice.preprocess.backend = "apple"`。定位：给**偏好 macOS 自带语音处理、且不介意每次
> 录音开头有较长停顿**的用户。不是默认（默认是 webrtc，见 [webrtc_backend.md](webrtc_backend.md)）。
>
> **代码**：`src/voice/engine.rs`（`AppleCapture` 采集边界）、`src/voice/apple_source.rs`
> （helper server 复用 / stop drain / kill-recycle）、`src/voice/apple_capture_helper.swift`
> （`VoiceProcessedCapture`）。**录音生命周期不变量**见 [voice.md](voice.md)。

## 是什么

cpal 之外的第二条采集路径：Swift helper 跑 `AVAudioEngine` + `setVoiceProcessingEnabled`，
由 macOS 提供 AEC / 降噪 / AGC，输出同样归一成 16k mono i16 交给 VAD/ASR。它是 webrtc
之外唯一带真 AEC 的路径，适合回声/外放/噪声大的环境。

代价：VP 每次录音都要建立 voice-processing 链路，**启动比 off/webrtc 明显慢**。用户接受
这个开头停顿换 Apple 的处理质量，才选它。

## 当前设计（已定，别回退）

- **纯 VP 采集，不做 raw→Apple mix/handoff/bridge。** 曾试过"VP 起来前先用 raw 补
  VAD/ASR、首帧到达后 handoff"，实测跨源拼接伤流式 ASR、效果不如纯 raw 也不如纯
  Apple——**决定不合并**。Apple 启动成功后只输出 Apple VP PCM；helper spawn / start
  失败则本次降级到 raw/off 并 warn。
- **复用 helper 进程，但每次录音新建 `AVAudioEngine`。** 复用进程省 spawn+TCC 延迟；
  engine 对象绝不复用——反复在同一 engine 上 toggle VP 会让 AudioUnit 累积进坏态
  （wedge），stop 后必须释放 engine。
- **kill/recycle 兜底。** 正常 stop 保留 reusable server 供下次复用；未正常 stop / drop /
  stop 超时则 kill helper 进程并清 reusable slot，防 wedge 跨录音存活。
- **不发布 retained audio**（留存的是处理后音频）。需要完整原始音频用 `backend = "off"`。

## 硬约束（改前必读）

- **VP 只在录音期间开。** 常驻 VP 会持续压低其他 App 的系统音频（ducking），体验不可接受。
- **cpal 与 Apple 共享同一物理设备**（macOS default input）。Apple 侧卡死会连带把 cpal
  raw 采集也静音——排障别只盯 Apple。
- **stop/drain 与 cpal 等价**：没有 raw tap，但 stop 阶段仍要把 helper 返回的 residual
  PCM 继续送 ASR，不能简化成 kill helper 或丢队列。

## 已知风险

进程内重建 engine 不保证 100% 清掉 CoreAudio HAL 层的 VP 残留（Swift 侧
`disableVoiceProcessing` 报错被吞）。kill/recycle 是这条的兜底；若仍偶发 wedge，可靠
"ready 后超时无 PCM → 记 poison、杀进程重开"进一步兜。不做 deadline 类自动兜底——wedge
偶发、无法按需复现，deadline 假正高、不可测，且救不了已经开始的这次录音。
