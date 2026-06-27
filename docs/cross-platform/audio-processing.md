# Audio Processing Research

本文记录后续独立推进 audio preprocessing 时需要继承的调研结论和接口设计。当前阶段不把
WebRTC/Sonora/RNNoise/SpeexDSP 接入主线；Windows 继续保留现有 VAD-only baseline，retained
audio 继续保留发布前 loudness normalization。

## 当前音频链路

shuohua 当前内部 canonical capture 格式是：

- 16 kHz。
- mono。
- i16 PCM。
- VAD/ASR 使用同一份 recorder PCM 的不同消费路径。

Windows 当前链路：

```text
mic -> 16k mono i16 PCM -> VAD-only adaptive gain copy -> Silero VAD
                      \-> raw PCM -> ASR provider
                      \-> raw temp WAV -> retained loudness normalization -> FLAC/M4A
```

macOS 当前链路：

```text
mic -> 16k mono i16 PCM -> raw copy -> Silero VAD
                      \-> raw PCM -> ASR provider
                      \-> raw temp WAV -> retained loudness normalization -> FLAC/M4A
```

重要边界：

- Windows VAD preprocessing 只处理发给 Silero 的 PCM 副本。
- ASR provider 当前仍收到 raw PCM。
- retained audio 保存路径不复用 VAD preprocessing，而是在发布前对临时 WAV 做一次离线
  loudness normalization。
- retained loudness normalization 是通用 `voice::audio::finish` 发布路径，不是 Windows-only。

## 缩写

- PCM: Pulse-Code Modulation，原始音频采样数据。
- APM: Audio Processing Module，音频处理模块；WebRTC APM 指实时语音前处理模块。
- AGC: Automatic Gain Control，自动增益。
- NS: Noise Suppression，降噪。
- AEC: Acoustic Echo Cancellation，回声消除。
- HPF: High-Pass Filter，高通滤波。
- VAD: Voice Activity Detection，判断是否有人声。
- ASR: Automatic Speech Recognition，语音识别。
- FFI: Foreign Function Interface，Rust 调 C/C++ 的接口边界。
- C ABI: C 二进制接口，用来把 C++ API 包成 Rust 可稳定调用的窄接口。

## 候选方案

### Off / passthrough

完全不处理输入音频。优点是行为最透明、风险最低；缺点是用户必须自己保证麦克风电平和环境足够好。

### Current baseline

当前 Windows baseline 在 VAD-only PCM 副本上做 RMS/peak gated adaptive gain。它解决了 Windows
麦克风输入电平偏低导致 Silero VAD 不稳定的问题，但算法范围很窄，不应继续扩展成复杂自研音频处理栈。

### Official WebRTC APM

WebRTC APM 是成熟度最高的产品级方向，能力包括：

- AGC / AGC2：统一不同麦克风输入电平。
- NS：降低环境噪声对 VAD/ASR 的干扰。
- HPF / DC removal：去除低频 rumble 和直流偏移。
- Limiter / level control：防止 AGC 后削波。
- Level estimation：未来可用于 doctor/diagnostics 提示用户输入过小、过大或噪声过高。
- AEC：对会议软件很重要；对 shuohua 语音输入不是第一优先级，除非未来需要处理扬声器回声。

官方 WebRTC APM 不是一次性函数，而是有状态的实时 frame processor。典型模型是创建一个
`AudioProcessing` instance，然后持续喂 10ms PCM frame。启用 AEC 时还需要 reverse stream
（扬声器播放流）；shuohua 第一阶段不需要 AEC。

工程判断：

- WebRTC 本身支持 Windows/macOS/Linux，但官方代码不是轻量 Rust crate。
- 如果走官方 WebRTC，应 pin 一个 Chromium/WebRTC milestone branch，不追最新。
- Rust 主线不应直接依赖 WebRTC C++ 类型；应通过一层 shuohua 自己维护的 C ABI shim 隔离。
- 平台差异主要在 native build/link/package，不应该泄漏到 voice engine。
- 这条路线成熟度最高，但构建、vendor、license notice 和版本升级维护成本最高。

已验证事实：

- Rust crate `webrtc-audio-processing 2.1.0` 默认在 Windows/MSVC 下寻找系统
  `webrtc-audio-processing-2` 和 `pkg-config`，不满足当前单二进制构建目标。
- 同 crate 的 `bundled` feature 在本机 Windows/MSVC scratch build 中调用 Unix `cp` 失败。
- 因此现有 Rust wrapper 不能直接作为 shuohua Windows 原生主线依赖；如果选官方 WebRTC，倾向于
  自己维护 vendor/build + C ABI shim。

### Sonora

Sonora 是 WebRTC Audio Processing 的 pure Rust port，目标集中在音频处理，不包含 WebRTC 网络、视频、
ICE、RTP、data channel 等 shuohua 不需要的功能。

其 README 声称：

- port 自 WebRTC Native Code M145。
- 覆盖 AEC3、NS、AGC2、HPF、SIMD、FFT/common audio。
- Windows x64、macOS arm64、Linux x86_64 等平台有 CI build/test。
- C++ reference test suite 通过 Rust backend 验证。
- License 为 BSD-3-Clause。

本机 scratch build 结论：

- `cargo add sonora@0.1.0 && cargo build --target x86_64-pc-windows-msvc` 通过。
- Sonora 最符合“Rust 主体、三端单二进制”的工程方向。

风险：

- 版本仍是 `0.1.0`。
- GitHub 体量小，star/fork 和市场使用量不足。
- 维护者集中度高，不能当成已经被大规模产品验证的基础库。
- 接入前必须做真实 API、CPU、延迟、长时间运行、Windows/macOS/Linux build 和录音 A/B 验证。

### RNNoise / SpeexDSP / DeepFilterNet

- RNNoise：成熟、小、BSD-style，适合降噪实验；但不是完整 AGC/AEC pipeline。
- SpeexDSP：老牌 C 库，提供 NS/AGC/VAD/AEC，但算法较老。
- DeepFilterNet：降噪质量强，但模型更重、实时成本和 ASR artifact 风险更高，不适合作为第一步默认链路。

这些可以作为备选 spike，但不应替代 WebRTC/Sonora 作为完整语音前处理方向的主要候选。

## 建议接口

voice engine 不应该直接依赖任何外部音频处理库。建议先落一个内部稳定边界：

```rust
pub(crate) trait AudioProcessingBackend {
    fn process_capture_i16(&mut self, input: &[i16], output: &mut [i16]) -> anyhow::Result<()>;
    fn reset(&mut self) {}
    fn backend_name(&self) -> &'static str;
}
```

内部约定：

- `input.len() == output.len()`。
- backend 可以要求 frame 为 10ms，但 frame 切分和不足帧缓存由 shuohua pipeline 负责，不让
  voice engine 或 recorder 知道 backend 细节。
- 第一阶段固定输入为 16 kHz mono i16 PCM；若未来选择在设备原始采样率或 48 kHz 处理，应作为
  audio preprocessing 专项设计，不顺手改 recorder/ASR 边界。

推荐 backend enum：

```rust
enum AudioProcessingBackendKind {
    Off,
    Baseline,
    WebRtcNative,
    SonoraExperimental,
}
```

推荐 pipeline：

```text
raw mic PCM
-> AudioProcessingPipeline
   -> vad_pcm
   -> asr_pcm
   -> retained_pcm
```

推进顺序：

1. `vad_pcm = processed`，`asr_pcm = raw`，retained audio 继续 raw temp WAV + finish-time loudness
   normalization。
2. 用真实录音 A/B 验证 processed audio 是否改善 ASR。
3. 只有 A/B 证明稳定收益后，才考虑让 ASR branch 使用 processed audio。
4. 保存音频是否使用 processed audio 也单独验证；给人听的自然回放不一定等于给机器识别的最佳输入。

## WebRTC C ABI 草案

如果走官方 WebRTC，应只暴露 shuohua 自己的窄 C ABI：

```c
typedef struct ShuoApm ShuoApm;

typedef struct ShuoApmConfig {
    int sample_rate_hz;
    int channels;
    int enable_high_pass_filter;
    int enable_noise_suppression;
    int enable_gain_controller;
    int enable_echo_cancellation;
} ShuoApmConfig;

ShuoApm* shuo_apm_create(const ShuoApmConfig* config);
void shuo_apm_destroy(ShuoApm* apm);
int shuo_apm_process_capture_i16(
    ShuoApm* apm,
    const int16_t* input,
    int16_t* output,
    int frames
);
void shuo_apm_reset(ShuoApm* apm);
```

Rust 侧只依赖 `shuo_apm_*`，不依赖 WebRTC C++ headers。WebRTC C++ API 变化时，改 C++ wrapper，不改
voice engine。

## 采样率策略

当前 16 kHz mono 对 Silero VAD 和 ASR 是合理规格。WebRTC APM 支持更高采样率，成熟实时通信系统常用
48 kHz；但 shuohua 的主业务是语音输入，不应为了 APM 先改变全链路规格。

后续专项评估时需要比较：

- device rate -> WebRTC APM -> 16 kHz ASR/VAD。
- device rate -> 16 kHz canonical PCM -> WebRTC APM。
- device rate -> 16 kHz canonical PCM -> current baseline。

目标是减少反复重采样，而不是盲目追随某个库的默认采样率。

## 验收门槛

任何新 backend 进入默认链路前，必须至少通过：

- Windows x64 debug/release build。
- macOS arm64 release build。
- Linux x64 build。
- 单二进制分发检查，或明确的 embedded runtime extraction 设计。
- CPU/latency smoke。
- 长时间运行 smoke。
- 至少一组真实麦克风 A/B：quiet room、low input level、keyboard/fan noise、remote desktop。
- VAD transition 对比：误触发、漏触发、进入 Idle/恢复 Recording 的体验。
- ASR 对比：字错率/主观质量不能退化。
- retained audio playback 对比：不能削波、不能明显泵音或噪声放大。

## 参考链接

- WebRTC Native Code development: <https://webrtc.github.io/webrtc-org/native-code/development/>
- WebRTC release notes: <https://chromium.googlesource.com/external/webrtc/+/master/docs/release-notes.md>
- WebRTC APM g3doc: <https://webrtc.googlesource.com/src/+/7c793a7dbe548735fe9e1d107e00d17937202f47/modules/audio_processing/g3doc/audio_processing_module.md>
- WebRTC AudioProcessing header: <https://webrtc.googlesource.com/src/+/0695df1a5990ffd02cff4f7b49a865d7085f9d0b/modules/audio_processing/include/audio_processing.h>
- tonarino/webrtc-audio-processing: <https://github.com/tonarino/webrtc-audio-processing>
- sonora: <https://github.com/dignifiedquire/sonora>
- RNNoise: <https://github.com/xiph/rnnoise>
- SpeexDSP: <https://www.speex.org/docs/>
- DeepFilterNet: <https://github.com/Rikorose/DeepFilterNet>
