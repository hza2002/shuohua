# Audio Processing Research & Design

> Archived research/design note. Apple voice processing is now implemented as the macOS audio
> preprocessing path; current behavior and invariants live in `docs/modules/voice.md` and code.
> WebRTC/Sonora content below is retained only as historical context for possible future
> cross-platform audio preprocessing work.

本文记录音频预处理的技术调研、架构设计和推进计划。当前阶段 Apple voice processing
已作为 macOS 可选 backend 落地；WebRTC APM 保留为 Apple 方案不够用时的后备，而不是
默认下一步。

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

macOS `backend = "off"` 链路：

```text
mic -> 16k mono i16 PCM -> raw copy -> Silero VAD
                        \-> raw PCM -> ASR provider
                        \-> raw temp WAV -> retained loudness normalization -> FLAC/M4A
```

macOS `backend = "apple"` 链路：

```text
mic -> AVAudioEngine voice processing -> 16k mono i16 PCM -> Silero VAD
                                                       \-> ASR provider
```

重要边界：

- Windows VAD preprocessing 只处理发给 Silero 的 PCM 副本。
- ASR provider 当前仍收到 raw PCM。
- retained audio 保存路径不复用 VAD preprocessing，而是在发布前对临时 WAV 做一次离线
  loudness normalization。
- retained loudness normalization 是通用 `voice::audio::finish` 发布路径，不是 Windows-only。
- **macOS `off` 走 cpal(CoreAudio HAL 裸输入)，不经过任何系统语音处理。**
  Voice Isolation 等系统麦克风模式是 **per-app、需 app 主动走 voice-processing 采集路径
  才会触发**，不会静默施加在 cpal 裸输入上。当前 shuohua 拿到的 macOS 音频是真 raw。
- **macOS `apple` 不复用 cpal**。它由 Swift capture helper 自持 AVAudioEngine input node，
  输出已处理后的 canonical PCM。当前 Apple backend 不发布 retained audio。

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
- VPIO: VoiceProcessingIO，Apple 的系统级语音处理音频单元(`kAudioUnitSubType_VoiceProcessingIO`)。
- TCC: Transparency, Consent, and Control，macOS 隐私权限框架(麦克风授权归它管)。

## 能力分解:共享 vs 分叉

WebRTC APM 不是一条可以中途打 tap 的流水线——它是一个有状态的黑盒 frame processor，
内部固定处理顺序(HPF → AEC → NS → AGC → limiter)，吐一份输出。因此不存在"公共阶段
算一次，VAD/ASR 各自再补一段"的中间分叉模型。现实只有两种形态：(A) 一个实例、一份配置、
一份输出喂两边；(B) 两个实例、两份配置、各自输出。

| APM 能力 | 对 Silero VAD | 对 ASR | 归类 |
|---|---|---|---|
| **HPF / DC removal**(去低频 rumble、直流偏移) | 有益，稳定能量判断 | 安全，轻微有益 | ✅ 共享 |
| **AGC / 电平归一**(跨设备、跨外接麦) | **强相关**——这才是 silero VAD 不稳定的根源 | 安全，轻微有益(provider 常自带归一) | ✅ 共享 |
| **AEC**(扬声器回声消除，非首要) | 有益(若有回声) | 有益(若有回声) | ✅ 共享 |
| **NS 降噪** | 激进降噪**有益**(更干净的语音/静音边界) | **双刃**——激进降噪引入 musical noise/失真，**可能抬字错率** | ⚠️ **分叉** |
| **Transient suppression**(键盘咔哒) | 有益 | 可能误削语音，需实测 | ⚠️ 分叉 |

结论：**真正分叉的只有 NS 的激进程度。** HPF + AGC + AEC 这组"前级调理"对两边都安全，
可以一份输出同时喂 VAD 和 ASR。

起手策略：**单实例、ASR 安全档(HPF+AGC，NS=Low/Off)，同一份输出喂 VAD 和 ASR。**
只有 A/B 证明 VAD 在这份温和处理下仍然误/漏触发，才加第二个 NS=High 实例专供 VAD。

## 候选方案

### Off / passthrough

完全不处理输入音频。优点是行为最透明、风险最低；缺点是用户必须自己保证麦克风电平和环境足够好。

### Current baseline

当前 Windows baseline 在 VAD-only PCM 副本上做 RMS/peak gated adaptive gain。它解决了 Windows
麦克风输入电平偏低导致 Silero VAD 不稳定的问题，但算法范围很窄，不应继续扩展成复杂自研音频处理栈。

### Apple Voice Processing(macOS 首选)

macOS 自带三层语音相关能力(来源:本机 SDK MacOSX26.5)：

**第一层:系统麦克风模式(`AVCaptureMicrophoneMode`，macOS 12+)**

- `Standard` — 标准 voice DSP。
- `VoiceIsolation` — 隔离人声、压制其它信号。
- `WideSpectrum` — 最少处理，收全场声音。
- 用户通过控制中心选择，app 侧只读(`activeMicrophoneMode`)。
- **只在 app 主动走 voice-processing 采集路径时生效**，不会施加在 HAL 裸输入(cpal)上。

**第二层:VoiceProcessingIO / AVAudioEngine voice processing(本方案采用的)**

- `kAudioUnitSubType_VoiceProcessingIO`(`'vpio'`)——系统级语音处理 I/O unit。
- 通过 `AVAudioEngine` 的 `setVoiceProcessingEnabled:` + input node 接入。
- 能力：**AEC**(消回声)+ **AGC**(调电平，默认开)+ 内建 NS。
- 暴露的可控项(来源:`AudioUnitProperties.h` + `AVAudioIONode.h`)：
  - `voiceProcessingAGCEnabled`——AGC 开/关(**唯一有意义的旋钮**，默认开)。
  - `voiceProcessingBypassed`——整体旁路(A/B 对照用，非用户配置项)。
  - `VoiceProcessingQuality`(0-127)——**已废弃**(macOS 10.7-10.9，iOS 3.0-7.0)。
  - ducking 配置——输入场景无关。
- **本质是近黑盒**：NS 强度调不了、quality 已废弃。可调的只有 AGC 开/关。
- 优势：零第三方构建、零 vendor、Apple 维护、与 Apple ASR 配套自然。
- 劣势：不可细调、无 raw tap(采集源自己产 processed，拿不到未处理副本)、
  macOS 独占、侵入采集路径(要换掉 cpal)。

本方案的实际接入(`src/voice/apple_capture_helper.swift`)：

- **只翻一个开关**：`input.setVoiceProcessingEnabled(true)`，其余 VPIO 旋钮
  (`voiceProcessingAGCEnabled` / `voiceProcessingBypassed` / mute / ducking)全保持默认
  (AGC 默认开)。没有「更优的隐藏配置」——VPIO 本就是黑盒整包，默认即设计意图。
- **重采样用 `AVAudioConverter`**(带抗混叠 SRC)把 VP 输出转成 canonical 16k mono i16，
  与 cpal 路径(rubato FFT)质量对齐；不要退回手写线性插值降采样(会引入混叠)。

**版本边界(重要，别和 Apple ASR 混淆)：**

- `backend = "apple"` 语音处理采集只要求 **macOS 15+**(app 基线)。底层 API 地板更低：
  `setVoiceProcessingEnabled` 为 macOS 10.15+，权限用的 `AVAudioApplication` 为 macOS 14+，
  都被 15 基线覆盖。
- **Apple 本地 ASR(`SpeechAnalyzer`)才需要 macOS 26+**；它与预处理 backend 解耦。
  因此 macOS 15–25 的用户配云端 ASR provider，照样能用 `backend = "apple"` 预处理。

**第三层:Speech 框架(识别，非预处理)**

- `SFSpeechRecognizer`、macOS 26 新增 `SpeechAnalyzer` / `SpeechTranscriber` /
  `DictationTranscriber` / `SpeechDetector`(一个端上 VAD)。
- `SFVoiceAnalytics`:jitter/shimmer/pitch/voicing 声学特征，适合做 doctor/diagnostics。
- 本层不做预处理，不在此方案范围内。

#### Apple VP 的关键限制

1. **绑死在自持采集的输入单元上。** 它不是一段可插拔的 PCM 处理器——它自己管采集、
   自己内部处理、吐处理后的 PCM。因此：
   - 无法把 cpal 采的 raw PCM 喂进 Apple 处理。
   - 无法拿到 raw tap(没有未处理的副本)。
   - retained audio 会变成处理后的音频。
2. **采集形态与 cpal 不同，但不影响下游。** 两者都吐 16k mono i16 帧，下游
   (VAD/ASR/retained)不知道也不关心源是 cpal 还是 AVAudioEngine。
3. **TCC 麦克风权限。** 已在本机验证 helper 采麦可工作；但该机器已有稳定版授权，
   不能单独证明首次安装用户的 TCC 归属。首次安装仍需真实分发验证。
4. **A/B 评估困难。** Apple 处理是采集期实时发生的，无法对同一段录音分别跑
   raw vs Apple 处理做配对对比。WebRTC APM 在这点上反而更优(录一次 raw，
   离线对同一份音频跑 raw vs APM)。

### Official WebRTC APM

WebRTC APM 是成熟度最高的产品级方向，能力包括：

- AGC / AGC2：统一不同麦克风输入电平。
- NS：降低环境噪声对 VAD/ASR 的干扰(四档可调：Low/Moderate/High/VeryHigh)。
- HPF / DC removal：去除低频 rumble 和直流偏移。
- Limiter / level control：防止 AGC 后削波。
- Level estimation：未来可用于 doctor/diagnostics 提示用户输入过小、过大或噪声过高。
- AEC：对会议软件很重要；对 shuohua 语音输入不是第一优先级。

WebRTC APM 是有状态的实时 frame processor，典型用法是创建一个 `AudioProcessing`
instance，持续喂 10ms PCM frame。启用 AEC 时还需要 reverse stream；shuohua 第一阶段
不需要 AEC。

#### WebRTC 内置 VAD——不建议替代 Silero

WebRTC 内含三套 VAD(来源:WebRTC native code)：

1. **GMM VAD**(`common_audio/vad`，`WebRtcVad_*`)——经典轻量，但老、精度不如 Silero。
   只接 8/16/32/48k mono i16、10/20/30ms 帧。
2. **`VoiceDetection`(APM 老接口)**——**已从现代 APM 移除**。
3. **RNN VAD**(`agc2/rnn_vad/`)——精度最高的神经网络 VAD，但**内嵌在 AGC2 里给增益控制用，
   现代 APM 不把它作为可查询的"有没有人声"公开 API 暴露**。

**判断：VAD 决策权继续留给 Silero。** WebRTC 只用它的清理能力(HPF/AGC/NS/AEC)，
不替代现有 VAD。VadPause 状态机依赖 Silero，换 VAD 成本高于收益。

#### 工程判断

- WebRTC 本身支持 Windows/macOS/Linux，但官方代码不是轻量 Rust crate。
- 如果走官方 WebRTC，应 pin 一个 Chromium/WebRTC milestone branch，不追最新。
- Rust 主线不应直接依赖 WebRTC C++ 类型；应通过 shuohua 自己维护的 C ABI shim 隔离。
- 平台差异主要在 native build/link/package，不应该泄漏到 voice engine。
- shuohua 只需要 `modules/audio_processing`(APM)和 `common_audio`(重采样/FFT)。
  RTP/SCTP/ICE/视频/data channel 全部用不上。
- **Mac-only 第一阶段不碰 WebRTC；它作为 Apple VP 粒度不够时的升级路径。**
  Apple VP 无法细调 NS level——一旦 A/B 发现"差一口气、降噪太狠/太轻"，
  就是跳 WebRTC APM 的信号。

已验证事实：

- Rust crate `webrtc-audio-processing 2.1.0` 默认在 Windows/MSVC 下寻找系统
  `webrtc-audio-processing-2` 和 `pkg-config`，不满足单二进制构建目标。
- 同 crate 的 `bundled` feature 在 Windows/MSVC scratch build 中调用 Unix `cp` 失败。
- 因此现有 Rust wrapper 不能直接作为 shuohua 原生依赖；如果选官方 WebRTC，倾向于
  自己维护 vendor/build + C ABI shim。

### Sonora

Sonora 是 WebRTC Audio Processing 的 pure Rust port(M145)。

- 优势：纯 Rust、单二进制友好、BSD-3-Clause。
- 风险：`0.1.0`、维护者集中度高、未被大规模产品验证。
- 定位：如果 Apple VP 不够且不想吃 WebRTC 构建成本，Sonora 是更轻的后备；
  但同样需要真实 A/B 验证。

### RNNoise / SpeexDSP / DeepFilterNet

- RNNoise：成熟、轻量、BSD-style，适合降噪实验；但不是完整 AGC/AEC pipeline。
- SpeexDSP：老牌 C 库，提供 NS/AGC/VAD/AEC，但算法较老。
- DeepFilterNet：降噪质量强，但模型更重、实时成本和 ASR artifact 风险更高。

这些可以留作备选 spike，但不应替代 Apple VP / WebRTC APM 作为主要方向。

## 架构设计

### 核心洞察：两类不可合并的采集源

| | 族 A：cpal + 外部处理器 | 族 B：自处理采集源 |
|---|---|---|
| 实例 | Off(identity)、Baseline、WebRTC APM | Apple VP(macOS 独占) |
| 跨平台 | 是(所有平台) | 否 |
| raw tap | **有**(cpal 采 raw，处理器可选) | **无**(采集即处理) |
| 形态 | cpal 录音 + 可选 PcmProcessor | 自持 AVAudioEngine，内置处理 |
| retained audio | 可存 raw | 只能存 processed |
| VAD/ASR 分叉 | 可(VAD 副本再加 NS) | 不可(一份输出) |

**这两族不可合并成统一的"PCM 进 / PCM 出处理器"抽象。** 统一它们的是输出契约：
**canonical 16k mono i16 处理后帧流。** 对下游(VAD/ASR/retained)而言，源是 cpal
还是 AVAudioEngine 完全透明。

### CaptureSource 缝

设计原则：**接口锚定在通用家族(cpal+处理器)上，让 Apple 作为受限特例去 conform。**
通用家族是所有平台 + Apple 上 WebRTC 路径都要用的，Apple 是只有 macOS 且能力更少的特例。
不让先建的那个例外反向塑形接口。

```
trait CaptureSource {
    // 启动采集；!Send 边界里建立(和现在 engine::run 一样)。
    fn start(...) -> Result<Self>
    where
        Self: Sized;

    // 实时消费 canonical 处理后帧。必须区分正常 EOF / 请求停止 / terminal error，
    // helper 崩溃、TCC denied、AudioEngine start 失败不能被吞成一次正常 stop。
    async fn recv(&mut self) -> Result<Option<Vec<i16>>, CaptureError>;
    fn try_recv(&mut self) -> Result<Option<Vec<i16>>, CaptureError>;

    // 停止与收尾语义必须等价于现有 RecordingStream：
    // stop 后仍要 drain residual + stop_delay_ms；retained audio 由 engine 显式
    // Publish/Discard，terminal error 路径不发布。
    fn stop(&mut self);
    async fn drain_after_stop(&mut self) -> Result<Vec<Vec<i16>>, CaptureError>;
    async fn finish_audio(&mut self) -> anyhow::Result<Option<PathBuf>>;
    async fn discard_audio(&mut self) -> anyhow::Result<()>;

    // 能力声明
    fn provides_raw_tap(&self) -> bool;
    fn backend_name(&self) -> &'static str;
}
```

三个实现：

| 实现 | 形态 | provides_raw_tap |
|---|---|---|
| `CpalSource { processor }` | 现有 cpal recorder + `PcmProcessor`(off=identity / webrtc / baseline) | true |
| `AppleVpSource` | spawn Swift capture helper(AVAudioEngine voiceProcessing)，从 stdout 读帧 | false |
| `TestSource` | 现有 `for_test`，吐预置帧 → 测试照常驱动 | true(可注入 raw) |

**下游(VAD/ASR/retained 路由、VadPause 状态机、finalize)一行不用改**——它消费的
还是 canonical 帧，不知道也不关心 backend。引擎按 `provides_raw_tap()` 能力分支——
retained audio、VAD/ASR 分叉等逻辑 key 在能力上，而非 `#[cfg(target_os)]`。

上面的 trait 是目标形状，不是当前实现。Apple 纵切片落地后，代码仍先用
`engine::CaptureStream` 枚举承接 cpal / Apple 两个真实实现；等 WebRTC 或更多 backend
出现，再从真实重复处提 trait。现有 `RecordingStream` 的 stop/drain/retained-audio 契约
仍是权威。

### 已落地实现：Apple 纵切片

当前 Apple backend 的实际形态：

- 共享基础设施在 `src/platform/macos/helper.rs`：helper 二进制发布(flock + 原子替换)、
  PCM frame 编解码。
- Swift capture helper 在 `src/voice/apple_capture_helper.swift`：AVAudioEngine
  voice processing → tap → 16k mono i16 PCM。
- Rust wrapper 在 `src/voice/apple_source.rs`：生产录音只走可复用 `--server` helper；
  one-shot helper 仅用于 capture smoke。
- `engine::CaptureStream` 只在 capture 边界区分 cpal / Apple；VAD、ASR、finalize、
  post/history 不感知 Apple 特例。

关键运行约束：

- helper server 可在 daemon 生命周期内空闲常驻，但麦克风只在录音中开启。录音停止后
  helper 保留、AVAudioEngine 停止，macOS 麦克风指示灯应熄灭。
- helper 是 daemon 子进程；Rust 侧所有 spawn 均设置 `kill_on_drop(true)`，Swift server
  也会在 stdin EOF 时退出。daemon stop/退出时不应留下 helper。
- stdout 同时承载少量 JSON 控制事件和二进制 PCM。协议约束是：ready/stopped 只出现在
  PCM 流边界；helper 通过 writer queue gate 保证 ready 先于任何 PCM frame。
- stop 必须 drain residual PCM。Apple stop 返回 residual frames 给 engine，engine 继续
  按现有 stop/drain 语义送 ASR，避免尾字丢失。
- PCM frame 有最大样本数上限，协议错位不能触发无界分配。
- server ready/start/stop 都有超时；server 不可用或 start 失败时清缓存并返回 recorder
  start error。生产路径不回退 one-shot，因为 one-shot stop 只能 kill 子进程，无法保证
  converter residual drain。

### Swift 子进程模式(复用现有 ASR helper 范式)

你们已有的 `apple_helper.swift` 模式——Swift 编成独立可执行、`include_bytes!` 嵌入、
flock 原子解包到 `~/.cache/shuohua/`、Rust ↔ Swift 走管道——经评估是接 AVAudioEngine
的最佳方式。

| | 现有 ASR helper | 新增 capture helper |
|---|---|---|
| 持有 | SpeechAnalyzer | **AVAudioEngine + voiceProcessing** |
| stdin | 收 PCM 帧 | (不用) |
| stdout | 发 JSON 转写事件 | **控制事件 + canonical 16k mono i16 帧**(PCM 复用 5 字节 framing) |
| 角色 | 消费 PCM | **生产 PCM** |

优势：AVAudioEngine 的实时 render callback、主 run loop、CoreAudio HAL、`!Send` 建立
全锁在 Swift 进程自己的 run loop 里，Rust 只从管道读字节——不再有 in-process 音频回调
和 Tokio 主线程的冲突。

约束：

- stdout 的 PCM frame 是实时数据通道；日志只走 stderr，结构化启动/错误事件要有明确
  framing，不能和 PCM 字节混在一起。
- AVAudioEngine tap/render callback 不做阻塞 I/O；stdout 写入、重采样、错误上报必须
  在非实时路径完成。
- Rust 侧要保留 helper terminal error，与用户请求 stop、正常 EOF 分开处理。

#### TCC 现状

- 现有 ASR helper 从 stdin 读 PCM，自己不采麦，从不触发 TCC。
- **capture helper 要采麦** → 触发 TCC。而 macOS 的麦克风授权按责任进程归属；
  一个解包在 `~/.cache`、ad-hoc 签名的子进程采麦，可能：① 自己弹独立授权框，
  ② 归到父进程 `shuo`，③ 静默失败。
- 已有 `src/platform/macos/permissions.rs`(objc2)给 in-process cpal 采麦管权限，
  但那是对 `shuo` 自己采麦；子进程采麦是另一个 TCC 主体。
- 本机 smoke 已证明 helper 路径能采麦，但不能证明首次安装用户的授权归属。
- 如果后续真实分发发现 TCC 很丑，再回退到 objc2 in-process Apple 采集；capture 边界不变。

## 配置设计

对齐现有 `[voice.vad] backend = "silero"` 的风格：

```toml
[voice.preprocess]
backend = "off"   # off | apple | webrtc
                  # apple 仅 macOS；非法组合在加载时报错
```

- `off` = 当前 raw 行为。
- `apple` = AVAudioEngine voice processing(macOS 独占)。
- `webrtc` = 预留值，当前加载可解析但运行时报未实现。
- **仅此一个用户可见字段。** default = `"off"`(无 A/B 证据前不动所有用户默认行为)。

后端内部参数(WebRTC 的 NS level、AGC target 等)初期不进用户 schema——
由开发者在代码里调成 preset，或放入后端私有文件(类比 `asr/<provider>.toml` →
`preprocess/webrtc.toml`)当高级调试入口。用户能做的有意义选择只有一个：用哪种 backend。
这与 Apple 自己对用户的 UX(黑盒、顶多一个开关)一致，也与 Silero VAD 的阈值由开发者
调好、不暴露给用户的做法一致。

真有一天某个参数值得给用户调，也走语义化选项(如 `denoise = "light"`)而非暴露
库的内部枚举(如 `ns_level = "kHigh"`)。

## 决策树

```text
使用 Apple VP(macOS only) 做真实录音体验验证
├─ 处理后 ASR 字错率不退、VAD 更稳   → 收工，一份流喂两边 ✅
├─ VAD 更稳但 ASR 退化              → Apple VP 不能提供 raw tap；默认回退 backend=off，
│                                      或转 WebRTC-on-cpal 细调。若要 VP 喂 VAD、
│                                      raw 喂 ASR，必须作为显式双采集实验处理
│                                      (双设备占用、时钟漂移、TCC、retained audio 选择)。
└─ VP 太粗 / TCC 不可接受 / 明确需要跨平台同等处理 → 再上 WebRTC APM(跨平台、可调)
```

## 验收门槛

### Apple VP(当前阶段)

- [x] TCC 麦克风权限：本机已验证子进程 helper 可采麦；首次安装归属仍需分发验证。
- [x] capture helper smoke 能启动并吐出合法 16k mono i16 PCM frame
      (`cargo test helper_capture_smoke_reports_ready_and_pcm -- --ignored --nocapture`，
      2026-06-28 本机通过)。注意：该机器已安装稳定版 shuohua 且已授予权限，
      此结果证明 helper 路径可工作，但**不能单独证明首次安装用户的 TCC 归属**。
- [x] 喂通 Silero VAD / ASR 主链路；用户已做多次真实录音验证。
- [x] helper server 复用后仅首次录音冷启动较慢；后续启动延迟可接受。
- [x] 麦克风生命周期：录音中指示灯亮，后处理和空闲 helper 阶段指示灯灭。
- [ ] retained audio 在 processed-only 下的行为可接受。
      当前 `backend = "apple"` 只接实时 VAD/ASR 帧流，暂不发布 retained audio。
- [ ] `shuo doctor --apple-capture-smoke` 诊断增强：显示 Apple 当前 `activeMicrophoneMode`
      (`Standard` / `VoiceIsolation` / `WideSpectrum`)和系统 input mute state，用于解释
      VAD/ASR 质量异常；只做诊断，不把系统麦克风模式做成 shuohua 配置。
- [ ] macOS arm64 release build。

### WebRTC 是否进入开发

WebRTC 不是 Apple 阶段后的自动下一步。只有出现以下至少一种信号，才值得投入：

- Apple backend 在真实使用中让 ASR 字错率退化，且 `off` 又不能满足 VAD 稳定性。
- Apple VP 的黑盒 NS/AGC 太粗，需要可调 NS level / AGC target。
- 需要在 Windows/Linux 上提供与 macOS Apple VP 类似的输入调理能力。
- 需要 raw tap + processed tap 同时存在，用于 retained audio、A/B 或 VAD/ASR 分叉。

进入 WebRTC 前的门槛：

- [ ] 明确要解决的失败案例，且 Apple/off 两条路径都不能满意。
- [ ] Windows x64 debug/release build。
- [ ] macOS arm64 release build。
- [ ] Linux x64 build。
- [ ] 单二进制分发检查，或明确的 embedded runtime extraction 设计。
- [ ] CPU/latency smoke。
- [ ] 长时间运行 smoke。
- [ ] 至少一组真实麦克风 A/B：quiet room、low input level、keyboard/fan noise、remote desktop。
- [ ] VAD transition 对比：误触发、漏触发、进入 Idle/恢复 Recording 的体验。
- [ ] **按 ASR provider 分别 A/B**。保留逐 provider 的 bypass 逃生口。
- [ ] ASR 对比：字错率/主观质量不能退化。
- [ ] retained audio playback 对比：不能削波、不能明显泵音或噪声放大。

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
- Apple VoiceProcessingIO: 本机 SDK `AudioUnitProperties.h` / `AVAudioIONode.h`(MacOSX26.5)
- Apple Speech framework: 本机 SDK `Speech.framework/Modules/Speech.swiftmodule`(MacOSX26.5)
- WebRTC VAD tutorial: <https://walterfan.github.io/webrtc_note/3.media/audio_vad.html>
