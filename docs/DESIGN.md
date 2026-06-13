# 技术设计

外观规范 + 关键设计决策 + 技术选型 + 目录结构 + 不变量 + 测试 + 安全。
持久化/线协议格式见 [SCHEMA.md](./SCHEMA.md)，CLI 见 [CLI.md](./CLI.md)，高层需求见 [REQUIREMENTS.md](../REQUIREMENTS.md)。

---

## 1. 外观规范

### 1.1 Liquid Glass 材质

经实物对比（24 种私有材质变体全跑过），**首选**：

- **变体 19 = `control`**：跟 macOS 系统控件/工具栏一致的玻璃感，透明度高
- **变体 11 = `bubbles`**：透明度更高、更"水珠"质感，作备选

选定方式：默认 19，配置项 `overlay.glass_variant` 允许覆写为 11 或其它（手动调试用）。

> 私有方法 `set_variant:` 在 macOS 26.5 上仍存在；selector 名稳定。出现破坏性变化时回退到 `NSVisualEffectMaterialHUDWindow`。
>
> 不上 App Store，私有 API 风险可控。

### 1.2 Overlay 设计

#### 容器 & 视图层级（不变）

- 顶层 `NSPanel`（borderless / 透明 / 无阴影 / level=NSStatusWindowLevel）
- 视图结构：`NSPanel → root NSView (圆角 mask) → NSGlassEffectView + 内容子视图 siblings`
- **内容作 glass 的兄弟节点**，**不**用 `glass.contentView = ...`（会触发 AppKit 二次磨砂，材质回退到 vibrancy）
- 显示位置可配置（top-left / top-center / top-right / center / bottom-* / cursor-screen 跟随）
- macOS 26 不可用时静默回退到 `NSVisualEffectMaterialHUDWindow`（不弹错误）

#### 两排布局（v1）

```
╭───────────────────────────────────────────────────────────╮
│ ● Recording · 3.2s · 84字          Xcode  ·  filler→llm   │  ← 第 1 排 状态条
│ 今天我想写一篇关于分布式系统的一致性算法的文章｜             │  ← 第 2 排 ASR 实时文字
╰───────────────────────────────────────────────────────────╯
   ↑ Liquid Glass variant=19
   ↑ ~520×80
```

第 1 排（左→右）：
- **状态点**：`Idle=灰` / `Connecting=橙` / `Active=红` / `Idle子状态(思考中)=蓝` / `Stopping=黄` / `Error=红闪`
- **状态文字**：跟点同步切换
- **时长**（mm:ss 或 N.Ns）+ **当前字数**
- 右对齐：**当前 App 名称** · **当前 chain summary**（比如 `filler→llm`）

第 2 排：
- 当前 partial 文本（实时变化）
- 已 definite 的 segments 拼在 partial 前面，淡色显示
- 句末闪烁光标条（0.8Hz）

#### 动画方案

| 元素 | 动画 | 实现 |
|---|---|---|
| 状态点颜色切换 | 200ms 缓动 | `CABasicAnimation` on `backgroundColor` |
| 状态文字切换 | 200ms cross-fade | `CATransition` fade |
| 时长/字数 | **不加动画**（频繁更新会抖） | 直接 setStringValue |
| App / chain 切换 | 200ms cross-fade | `CATransition` fade |
| 第 2 排 partial 替换 | **瞬时切**（partial 本来就在被覆盖） | setStringValue 直接刷 |
| 新字符追加 | 尾部 1-2 字浅黄高亮，300ms 淡出 | NSAttributedString + CATransaction |
| segment 定型 | 一闪淡绿 200ms → 默认色 | 同上 |
| 句末光标条 | 50% 占空比闪烁，0.8Hz | NSTimer 翻 hidden |
| Overlay 整体出现/消失 | 弹簧动画 ~250ms | `NSWindow.alphaValue` + spring |

#### Overlay 内部状态接口（同进程，channel 驱动）

由于单进程，原"daemon → helper JSON"改为 tokio 后台线程 → AppKit 主线程的内部消息。
view model 字段如下（用 `tokio::sync::mpsc` 把变化推到主线程，AppKit 端 merge + 跑动画）：

```rust
enum OverlayCmd {
    SetState  { state: OverlayState, color: u32, label: String },
    SetStats  { dur_ms: u64, chars: u32 },
    SetApp    { bundle_id: Option<String>, chain_summary: String },
    SetText   { text: String, kind: TextKind },     // Partial / Final
    AppendSegment { text: String },
    Toast     { text: String, level: ToastLevel, ttl_ms: u32 },
    Hide,
}
```

**Toast UI**：用 Liquid Glass 同款样式（同变体、同圆角、同字体）的一条小胶囊，**底部弹出**（不覆盖第 2 排的 ASR 文字），1.5s 自动消。和主胶囊浮在同一窗口，避免另开 NSPanel。用来报 PostProcessor 失败、网络重连这类不影响主流程的事件。

### 1.3 进程模型

#### 单进程，AppKit 主线程 + tokio 后台线程

参考 electron-liquid-glass 的思路：NSGlassEffectView 在宿主进程内嵌即可，**不拆子进程**。daemon 单进程承担所有职责：

| 线程 | 职责 |
|---|---|
| **主线程**（AppKit / CFRunLoop） | NSPanel + NSGlassEffectView 渲染；`NSApplication.run()` |
| **专用 OS 线程**（CFRunLoop） | CGEventTap 热键拦截（不让出，§5 不变量 1） |
| **tokio 多线程 runtime** | 录音、VAD、ASR、PostProcessor、UDS server、history 写入、配置热重载 |

进程拆分：

| 进程 | 何时跑 | 职责 |
|---|---|---|
| **daemon**（`shuo --daemon`，或 `shuo` 智能 fallback 时起） | launchd 开机自启，常驻 | 上述所有 |
| **TUI 客户端**（`shuo` 检测到 daemon 存在时） | 用户按需启动 | 连 UDS 看实时状态、滚动历史；**关掉不影响 daemon** |

#### Daemon ↔ TUI 通信：UDS + 文件双通道

| 通道 | 用途 | 性能特性 |
|---|---|---|
| `/tmp/shuohua-${UID}.sock`（UDS） | 实时状态流 + 控制命令 | **TUI 不连接时 daemon 零 UI 开销**；连上时事件驱动 push，无 polling |
| `${XDG_STATE_HOME:-~/.local/state}/shuohua/history.jsonl` | 识别历史 append-only | TUI 启动时读一次算统计；外部脚本/未来 GUI 也读它 |
| `${XDG_STATE_HOME:-~/.local/state}/shuohua/log.jsonl` | 结构化日志 | tracing 写；TUI 可滚动查看 |

UDS 协议格式见 [SCHEMA.md](./SCHEMA.md)。

---

## 2. 关键设计决策

### 2.1 Voice 状态机用 enum，不用 11 个 bool

```rust
enum Voice {
    Idle,
    Connecting { recording: RecordingId, cancel: CancellationToken },
    Recording {
        recording:      RecordingId,
        cancel:         CancellationToken,        // ← 父 token；停止时 cancel 整棵子树
        sub:            RecordingSub,             // ← Active / Idle 子状态
        pending_output: String,                   // ← 跨多次 ASR session 文本累积
        last_voice_at:  Instant,
        started_at:     Instant,
    },
    Stopping  { recording: RecordingId },          // 用 Recording.cancel 取消
    Finishing { recording: RecordingId },
    Error     { until: Instant, last: Option<RecordingId> },
}

enum RecordingSub {
    Active { asr: Box<dyn AsrSession>, cancel: CancellationToken },  // ← root.child_token()
    Idle,    // ASR 已关，麦克风仍在听（client VAD 等下一段 voiced）
}
```

非法状态（如"录音中但 stopping=true"）编译期被消除。

**结构化并发**：`Recording.cancel` 是 root；`Active.cancel = root.child_token()`。Stopping 时只 cancel root，ASR / dispatch / pipeline 所有子 task 一起收，无需手动逐个 cancel。

**子状态独立于用户 toggle**：用户保持 toggle ON 期间，daemon 可以在 Active ↔ Idle 之间自动来回（详见 §2.9）。

只有 toggle 模式（hold 模式不做），所以**没有 Mode 字段、没有 hold_released 字段**。

### 2.2 双输出 bug 用 type-state 永久消掉

识别结果同时从 `Final` 和 `Done` 两条 channel 上来，必须只输出一次。Rust 里：

```rust
struct OutputToken(RecordingId);  // 不 Clone
fn dispatch(token: OutputToken, text: String) { /* 消费掉 */ }
```

谁先到谁拿走 token，编译器保证另一条无法二次派发。Go 的运行时锁 `claimSessionOutput` 升级为编译期保证。

### 2.3 结构化并发取消，消掉 `sessionGen` 世代标志

录音收尾五步链（recorder stop → final send → ASR final wait → dispatch → ASR close）挂在一棵 `tokio::task` 树上，通过 `CancellationToken` 一次取消全链。Go 的 sessionGen / sessionID 双计数器不再需要。

### 2.4 真正实现热键 Suppress

CGEventTap C 回调里通过 `arc_swap::ArcSwap<RegisteredCombos>` 读注册表的不可变快照（无锁、无分配），决定 `return null` 还是 `return event`。toggle 模式下也可以真正吞掉热键的 keydown，避免泄漏给前台 App（见 §5 不变量 down/up 配对吞）。

### 2.5 去掉 Plugin 抽象

只有 voice / overlay / debug 三个固定模块，不抽 `Plugin` trait。直接 module 内 `tokio::spawn`，配置变化通过 `tokio::sync::watch::Receiver<Arc<Config>>` 广播。

### 2.6 错误用 thiserror 结构化

`AsrError::Timeout` / `AsrError::Auth` / `AsrError::Network` 而不是字符串。TUI 和 doctor 按类型给具体建议。

### 2.7 Hotkey tracker 用 proptest

纯函数式状态机，property test 断言 KeyDown/KeyUp 配对等不变量。

### 2.8 ASR Provider 抽象

明确计划支持多家 ASR，从 v1 就抽 trait。**接口按 voice 模块的语义需要设计**，不按某家 provider 的协议反推。

**硬契约**：
- 流式 partial 是必需的，不是可选 cap。不支持原生流式或不能包装成流式的 provider 直接不入选
- **单事件流**：partial / segment / done 走同一根 channel（`AsrEvent` enum），voice 模块只 select 一臂
- provider 私有配置类型化分离在自己的 TOML 段，voice 模块永远不见
- provider 必须保证：`send_pcm(is_last=true)` 之后**至少**会出一个 `AsrEvent::Segment`，然后 `AsrEvent::Done`

```rust
#[async_trait]
pub trait AsrProvider: Send + Sync {
    fn name(&self) -> &str;
    fn caps(&self) -> Caps;
    async fn open(&self, ctx: SessionCtx) -> Result<(Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>)>;
}

pub struct Caps {
    pub hotwords:         bool,       // false → ctx.hotwords 静默忽略，doctor 会提示
    pub max_session_secs: Option<u32>,
    pub multilingual:     bool,       // 是否同 session 内 code-switch
}

pub struct SessionCtx {
    pub language: LanguageMode,
    pub hotwords: Vec<Hotword>,
}

pub enum LanguageMode {
    Single(String),                       // "zh-CN" / "en-US"
    Multilingual { hint: Vec<String> },   // 中英混合走这个
}

pub struct Hotword {
    pub word:  String,
    pub boost: Boost,                     // enum 三档；provider 映射到自家区间
}

pub enum Boost { Low, Medium, High }      // TOML 里写 "low" / "medium" / "high"

#[async_trait]
pub trait AsrSession: Send {
    /// 喂 PCM。is_last=true 表示后面没了，provider 必须在收到后吐 Segment + Done。
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<()>;

    async fn close(self: Box<Self>) -> Result<()>;
}

/// 单事件流。voice 模块 select 这根 channel 就够了。
pub enum AsrEvent {
    Partial { text: String, seq: u64 },                                 // 当前 utterance 最新猜测全文
    Segment { text: String, started_at: Instant, ended_at: Instant },   // 句末（server VAD 或 is_last 后）
    Error   { kind: AsrErrorKind, msg: String },                        // 不要混进 Result，让 voice 决定降级
    Done,                                                               // session 终结（is_last + 最后一段已发完）
}
```

**为什么删 `server_side_vad` cap**：原本用来告诉 voice "session 中间会不会冒 Segment"。改用单事件流后，voice 行为统一（来什么处理什么），不需要分支。Provider 实现保证 `is_last=true` 后至少出一个 Segment 即可。

**Boost enum 三档**：原浮点 0.0–1.0 各家 provider 映射规则不同（Doubao 整数档位、Deepgram 1.0–10.0、AssemblyAI 无 boost），用户写 `0.8` 猜不到效果。三档 enum 直观，TOML 写 `boost = "high"`。

#### 各 provider 怎么映射

| Provider | Partial | Segment | 备注 |
|---|---|---|---|
| Doubao SAUC | `definite=false` → `AsrEvent::Partial` | `definite=true` → `AsrEvent::Segment` | M2 首发 |
| GPT-4o-Transcribe Realtime | `.delta` 事件 | `.completed` 事件 | 中英混合最佳候选 |
| Apple SpeechAnalyzer (macOS 26) | `isFinal=false` 委托 | `isFinal=true` 委托 | objc2 适配；M9 评估中文 |
| whisper.cpp 流式包装 | 每 ~300ms 局部重跑 | 客户端 VAD 切段 | M8 离线方案 |
| Deepgram / AssemblyAI | `is_final=false` | `is_final=true` | 未来 |
| ~~OpenAI Whisper API (batch)~~ | 无 | 无 | **不入选** |

#### TOML 配置策略

每个 provider 一段独立 TOML 类型化解析，voice 模块只拿 `Box<dyn AsrProvider>`：

```toml
[asr]
provider = "doubao"

[asr.doubao]
app_key         = "..."
access_key      = "..."
resource_id     = "volc.bigasr.sauc.duration"
end_window_size = 3000
language        = "multilingual"
hotwords_path   = "~/.config/shuohua/tech_hotwords.toml"

[asr.whisper_cpp]                    # 未来 M8
model_path = "/path/to/ggml-large-v3-q5_0.bin"
language   = "zh"
threads    = 4
initial_prompt = "以下文本包含 Rust、tokio、Kubernetes 等技术术语"
```

热词单独一份 TOML 文件，方便维护：

```toml
# tech_hotwords.toml
[[hotword]]
word  = "Rust"
boost = "high"

[[hotword]]
word  = "tokio"
boost = "high"

[[hotword]]
word  = "Kubernetes"
boost = "medium"
```

### 2.9 客户端 VAD + 多段 session（"思考不计费"机制）

#### Doubao 计费模型（核实后的事实）

Resource ID `volc.bigasr.sauc.duration` 的 `duration` 是关键：**按音频时长计费**（约 3.5 元/小时，0–300h 阶梯）。默认免费 10 路并发，单用户单连接不会触发限额。

**省的不是"会话数"，而是这段静默时间不被作为音频喂给 Doubao**。10 分钟"在想"如果一直喂静音 PCM，扣 10 分钟时长费（≈0.58 元）；client VAD 检测到静音不喂，就 0 费。

详细：[计费说明](https://www.volcengine.com/docs/6561/1359370?lang=zh) / [大模型流式 API](https://www.volcengine.com/docs/6561/1354869?lang=zh)。

#### 两种实现方案

| 方案 | 做法 | 优点 | 缺点 |
|---|---|---|---|
| **A. 关 session**（v1 选） | 静音 ≥3s → close → 静音期不连 → 恢复说话开新 session | 简单，不需要心跳，Doubao 行为完全确定 | 重连首字延迟 ~300–500ms |
| **B. 保 session 暂停喂** | 静音 ≥3s → 不发 PCM（连接保持）→ 恢复说话直接喂 | 首字延迟 ~200ms | 需心跳防 server 端超时；Doubao 文档没说静音保持上限，需实测 |

**v1 = A**。M3 之后实测首字延迟刺眼再切 B。

#### 两个独立的静音阈值

```toml
[voice]
pause_asr_silence_ms = 3000      # 静音 3s → 关 ASR session，仍保持 Recording 状态
auto_stop_silence_ms = 600000    # 静音 10min → 完全 stop（防忘按）
segment_separator    = " "       # 段间用什么拼
```

约束：`pause_asr_silence_ms ≥ end_window_size + 1000`，确保客户端先于服务端 VAD 触发。

#### 子状态机转移

| 当前 | 事件 | 下一个 |
|---|---|---|
| `Recording.Active` | unvoiced 持续 ≥ pause_asr_silence_ms | `Recording.Idle`（关 ASR，把 final 追加到 pending_output） |
| `Recording.Idle` | VAD 检测到 voiced | `Recording.Active`（开新 ASR session；先 dump VecDeque 再喂当前帧） |
| `Recording.*` | unvoiced 持续 ≥ auto_stop_silence_ms | `Stopping` |
| `Recording.*` | toggle OFF / cancel / 错误 | `Stopping` |

#### VAD 实现

| 候选 | 准确度 | CPU | 选 |
|---|---|---|---|
| **WebRTC VAD**（`webrtc-vad` crate / libfvad） | 中等，业界 workhorse | <0.1% | **v1 默认** |
| RMS 阈值 | 低，键盘/风扇容易误触 | 几乎 0 | 降级路径 |
| Silero VAD（ONNX） | 高 | ~5% (M1) | M9 备选 |

WebRTC VAD 工作方式：
- 16kHz mono PCM 切 **20ms 帧**（每帧 320 samples）
- 每帧返回 speech / not-speech bit
- **滑动窗口去抖**：最近 5 帧（100ms）至少 3 帧 speech 才算 voiced，反之类似
- **切换最小间隔 ≥ 1s**：防打字声触发 ON/OFF 抖动

#### PCM 数据流（单消费者 + 滑窗历史）

为了避免多消费者并发问题，PCM 数据流只有一个消费者任务：

```
cpal callback ──► SPSC ring buffer (rtrb) ──► PcmConsumer task
                                                  │
                                                  ├─► 维护 500ms VecDeque<i16> 历史（滑窗）
                                                  ├─► WebRTC VAD 判定每帧 voiced/unvoiced
                                                  └─► 当 ASR session active 时喂 PCM
```

关键点：
- **cpal callback** 唯一写入端，直接送 ring buffer（lock-free SPSC，`rtrb`）。callback 内不做任何阻塞工作（不分配、不日志）。
- **PcmConsumer** 是唯一读取端：取 20ms 帧 → 跑 VAD → 推入 `VecDeque<i16>`（容量 = 500ms = 8000 samples），超出从头 pop。
- **Idle → Active 触发**（VAD 检测到 voiced + 当前无 ASR session）：
  1. 异步 spawn 开新 ASR session 的 task（用 `tokio::sync::oneshot` 通知 PcmConsumer "session ready"）
  2. PcmConsumer 把 VecDeque 全部历史（500ms）一次性喂给新 session
  3. 继续把当前帧及后续帧喂给 session
- **VecDeque 始终维护**，不因 ASR 状态变化重置 —— 这样下一次 Idle→Active 仍然有 pre-roll 可用。

为什么 500ms：
- <200ms：辅音/弱起会丢（"今天" → "天"）
- >800ms：增加无效计费且无收益
- 500ms 在中英文都验证过

**对计费的真实影响**：每段开头多喂 500ms。一次录音 3 段 = 多 1.5s = 0.0015 元。可忽略。

#### cpal / VAD 生命周期（跟随 Voice 状态机）

为了满足空闲 CPU <0.5% + 隐私目标，**cpal 和 PcmConsumer 不是常驻**：

| Voice 状态 | cpal | PcmConsumer | VecDeque 历史 |
|---|---|---|---|
| `Idle`（没按 F9） | **关** | **关** | — |
| `Connecting` → `Recording.*` | 开 | 开 | 滑窗维护 |
| `Stopping` → `Finishing` | drain 后关 | 收尾后关 | 丢弃 |

**F9 触发流程**（Idle → Connecting）：
1. 主 voice 任务收到 toggle ON 事件
2. 启动 cpal stream（macOS 上 ~50ms 初始化）
3. 启动 PcmConsumer task
4. 进入 Connecting 状态等 ASR 连接

**这意味着第一次 F9 按下后的最初 ~50ms PCM 可能不可用**——但用户从按 F9 到开口说话的反应时间通常 200ms+，所以听感上无损失。pre-roll 的真正价值在 Recording 内的 Idle→Active 切换（思考停顿后恢复说话），那时 cpal 已经在跑、VecDeque 已经满。

**Stopping**：cpal 收到 stop 信号后继续 drain `stop_delay_ms`（800ms，见 §5 不变量）确保尾字不丢，然后才真正关流。

#### 关键不变量

1. **VAD 独立于 provider**：任何 provider 不需要关心 VAD。
2. **段间分隔符**默认空格，可配。
3. **PcmConsumer 不阻塞**：开/关 ASR session 是异步任务（spawn 到 tokio runtime），PcmConsumer 继续读下一帧。session 没 ready 时帧累积在 VecDeque + ring buffer 里。
4. **cpal 跟随 Voice 状态机**：Voice::Idle 时绝不持有 cpal stream（防 always-recording 隐私问题 + 满足 <0.5% 空闲 CPU）。

### 2.10 PostProcessor 抽象

#### 数据形态

```rust
/// 流过整条链的数据。raw 永不变（保留回退/记录），text 是当前正被加工的版本。
pub struct PipelineText {
    pub raw:      String,           // 原始 ASR 拼接，整条链不变
    pub sessions: Vec<AsrSessionRecord>,  // 多段 ASR session 的记录（带时间戳，对应 history.asr.sessions）
    pub text:     String,           // 当前 in-flight 版本（上一个 processor 的输出）
}

/// 前台 App 上下文。daemon 在 toggle OFF 时取一次，整个链共享。
pub struct AppContext {
    pub bundle_id: Option<String>,  // "com.apple.dt.Xcode"
    pub app_name:  Option<String>,  // "Xcode"
}
```

#### Trait

```rust
#[async_trait]
pub trait PostProcessor: Send + Sync {
    fn name(&self) -> &str;
    async fn process(
        &self,
        input: PipelineText,
        ctx:   &AppContext,
    ) -> Result<PipelineText, PostError>;
}
```

#### 链式执行（超时 + 失败处理）

```rust
async fn run_chain(
    chain: &[Box<dyn PostProcessor>],
    initial: PipelineText,
    ctx: &AppContext,
    timeout: Duration,
    toast_tx: &mpsc::Sender<ToastEvent>,
    step_tx: &mpsc::Sender<PipelineStep>,   // 实时推 UDS pipeline_step 事件
) -> (PipelineText, Vec<PipelineStep>) {
    let mut current = initial;
    let mut steps  = vec![];
    for p in chain {
        let started = Instant::now();
        let step = match tokio::time::timeout(timeout, p.process(current.clone(), ctx)).await {
            Ok(Ok(out)) => {
                let s = PipelineStep::ok(p.name(), started.elapsed(), out.text.clone());
                current = out;
                s
            }
            Ok(Err(e)) => {
                let _ = toast_tx.send(ToastEvent::warn(format!("{} failed, skipped", p.name())));
                PipelineStep::err(p.name(), started.elapsed(), e.to_string())
            }
            Err(_) => {
                let _ = toast_tx.send(ToastEvent::warn(format!("{} timed out, skipped", p.name())));
                PipelineStep::timeout(p.name(), started.elapsed())
            }
        };
        let _ = step_tx.send(step.clone());
        steps.push(step);
    }
    (current, steps)
}

/// 对应 history.jsonl 里 pipeline[] 的每一项，也对应 UDS pipeline_step 事件。
#[derive(Clone)]
pub struct PipelineStep {
    pub name:        String,
    pub status:      StepStatus,             // Ok / Error / Timeout / Skipped
    pub duration_ms: f64,
    pub text:        Option<String>,         // Ok 才有
    pub error:       Option<String>,
}
```

**两条规则**：
- **链不阻塞，最差是 raw**：失败/超时跳过该步，下一个继续用 upstream 的 text。不假设"后面会补"——后面的 processor 各干各的活，跳过的工作就是丢了。链路始终产出（最差等于 raw）
- **失败/超时都推 toast**：用户看得见，但流程不阻塞
- 所有失败 + 时延都写进 `pipeline[]`，进 history.jsonl + UDS 事件

#### 按 App 分离的配置（文件树）

```
~/.config/shuohua/
├── config.toml                            # 全局
└── post/
    ├── default.toml                       # 默认链
    └── app/
        ├── com.apple.dt.Xcode.toml        # Xcode 专属
        ├── com.tinyspeck.slackmacgap.toml
        ├── com.microsoft.VSCode.toml
        └── ...                            # 用户随时新增
```

匹配逻辑：toggle OFF 时取 `frontmost_bundle_id`，去 `post/app/<bundle_id>.toml` 找；找到就用，找不到 fall back 到 `post/default.toml`。`notify` watch 这个目录，新增/删除/修改实时生效。

主 config.toml 只指路：

```toml
[post]
default_chain_path = "~/.config/shuohua/post/default.toml"
per_app_dir        = "~/.config/shuohua/post/app/"
timeout_ms         = 2000     # 单步 processor 超时
```

每份链文件长这样（示例：Slack）：

```toml
# post/app/com.tinyspeck.slackmacgap.toml
name  = "Slack 偏 casual"
chain = ["filler", "llm_casual"]

[processors.filler]
type     = "rule_based"
patterns = ["嗯", "啊", "呃", "那个", "就是"]
collapse_repeats = true

[processors.llm_casual]
type        = "llm"
provider    = "anthropic"
model       = "claude-haiku-4-5"
api_key_env = "ANTHROPIC_API_KEY"
prompt = """
清洗成 Slack 聊天风格的中文/英文混合文本。保留口语化。
只输出清洗后的文本，不要解释。
"""
```

#### 内置 processors（v1）

| 名字 | 类型 | 何时引入 | 作用 |
|---|---|---|---|
| `IdentityProcessor` | 内置 | M2 | 透传，等于关后处理 |
| `RuleBasedFiller` | 内置 | M2.5 | regex 去 嗯/啊/呃/那个/就是；可选合并重复字 |
| `LlmCleanup` | 内置 | M7 | 调 OpenAI 兼容 API（Anthropic / OpenAI / 任意兼容端点）|

`LlmCleanup` 的 prompt 接受变量替换：`{{app_name}}` / `{{bundle_id}}` / `{{text}}` —— 用户可以在 prompt 模板里引用。

#### 跟 UDS / history 的对接

- **链路进行中**：每完成一步 processor，daemon 推一条 `pipeline_step` 事件到 UDS。TUI 实时显示每步 status + duration + text。
- **链路结束 + dispatch 完成**：daemon 把整条 `HistoryRecord`（schema 见 [SCHEMA.md](./SCHEMA.md)）append 到 history.jsonl，同时推一条 `history_appended` 事件。**不再有独立的 `final` 事件**——会话完成即 history_appended。
- **TUI 默认显示完整流水线**：raw → 每个 processor 的 output → 最终上屏文本。不切换模式，全可见，方便观测整条链。
- **Overlay 只显示最终上屏文本**（chain 最后一项的 text；chain 空则 = raw），失败/超时通过 toast 通知。

#### 不做的事（v1）

- 不做内容审查（敏感词、政治、隐私过滤都不做）—— PostProcessor 只清洗不审查
- 不做 per-URL / per-input 字段的更细粒度匹配 —— 粒度到 bundle_id 为止
- 不允许 processor 整段拒绝输出 —— 失败只能跳过该步，链路始终产出（最差是 raw）

### 2.11 i18n（界面双语）

v1 支持 **zh-CN** 和 **en-US**。设计目标：**不引第三方 crate**（`rust-i18n` / `fluent-rs` 都过重），手写 ~100 行搞定。

#### 字典文件结构

```
assets/i18n/
├── zh-CN.toml
└── en-US.toml
```

```toml
# assets/i18n/zh-CN.toml
[overlay]
state_idle       = "空闲"
state_recording  = "录音中"
state_connecting = "连接中"
state_stopping   = "收尾"
state_error      = "错误"

[doctor]
ok_accessibility = "Accessibility 权限：OK"
err_microphone   = "Microphone 权限：缺失，请到系统设置 → 隐私 → 麦克风 中授权"

[toast]
asr_timeout      = "ASR 超时，已跳过"
llm_failed       = "{name} 失败，已跳过"   # 支持 {var} 占位符
```

```toml
# assets/i18n/en-US.toml
[overlay]
state_idle       = "Idle"
state_recording  = "Recording"
state_connecting = "Connecting"
state_stopping   = "Stopping"
state_error      = "Error"

[doctor]
ok_accessibility = "Accessibility permission: OK"
err_microphone   = "Microphone permission: missing. Grant in System Settings → Privacy → Microphone"

[toast]
asr_timeout      = "ASR timeout, skipped"
llm_failed       = "{name} failed, skipped"
```

#### 实现（约 80 行）

```rust
// src/i18n/mod.rs
use std::collections::HashMap;
use std::sync::OnceLock;
use arc_swap::ArcSwap;

static DICT: OnceLock<ArcSwap<Dict>> = OnceLock::new();

pub struct Dict {
    pub lang:    Lang,
    pub entries: HashMap<String, String>,   // 扁平 key 如 "overlay.state_idle"
}

pub enum Lang { ZhCN, EnUS }

pub fn init(cfg_lang: &str) {
    let lang = resolve_lang(cfg_lang);     // "auto" → 读 $LANG → fallback en-US
    let entries = load_toml(lang);
    DICT.set(ArcSwap::from_pointee(Dict { lang, entries })).ok();
}

/// t!("overlay.state_recording") → "录音中" / "Recording"
/// t!("toast.llm_failed", name = "filler") → "filler failed, skipped"
#[macro_export]
macro_rules! t {
    ($key:expr) => { $crate::i18n::tr($key, &[]) };
    ($key:expr, $($k:ident = $v:expr),*) => {
        $crate::i18n::tr($key, &[$((stringify!($k), $v.to_string())),*])
    };
}

pub fn tr(key: &str, vars: &[(&str, String)]) -> String {
    let d = DICT.get().unwrap().load();
    let template = d.entries.get(key).cloned().unwrap_or_else(|| key.to_string());
    vars.iter().fold(template, |acc, (k, v)| acc.replace(&format!("{{{}}}", k), v))
}
```

#### 配置

```toml
[ui]
language = "auto"        # auto | zh-CN | en-US
```

`auto` 解析逻辑：
1. 读 `$LANG`
2. 以 `zh` 开头 → `zh-CN`
3. 其他 → `en-US`

#### 应用范围

| 出口 | 是否走 i18n |
|---|---|
| Overlay 文本（状态点 label、toast） | ✓ |
| TUI 全部文本 | ✓ |
| Doctor 输出 | ✓ |
| `shuo help` / clap 自动文案 | 默认 en-US（clap 自带不易换；help 实际上是英文用户也能看懂的） |
| 日志（tracing） | en-US 固定（debug 用，不走 i18n） |
| history.jsonl | 不本地化（数据格式） |

#### 热重载

`ui.language` 配置项跟随 config.toml 整体热重载（`notify` 监听）。语言切换瞬时生效，无需重启 daemon。

---

## 3. 技术选型

| 用途 | crate | 理由 |
|---|---|---|
| Objective-C 互操作 | `objc2` 0.6 + `objc2-app-kit` 0.3 + `objc2-foundation` 0.3 | 现役标准，活跃维护 |
| Core Graphics / CGEventTap | `core-graphics`, `core-foundation` | CGEventTap pipe 桥的基础 |
| 录音 | `cpal`（首选）/ `coreaudio-rs`（备选） | cpal 简单，coreaudio-rs 控制更细。先用 cpal |
| VAD | `webrtc-vad`（libfvad 绑定） | 业界 workhorse，<0.1% CPU；失败降级到 RMS |
| PCM ring buffer | `rtrb` | SPSC，cpal 回调到 PcmConsumer |
| 唯一 ID | `ulid` | history record id；26 字符短于 UUID，含时序信息 |
| WebSocket | `tokio-tungstenite` | tokio 生态首选；DoubaoProvider 用 |
| TUI | `ratatui` + `crossterm` | Bubble Tea 的事实替代；**唯一前台 UI** |
| TOML | `toml` + `serde` | 标准 |
| 文件监听 | `notify` | 监听**目录**而非文件（避免 inode 替换） |
| 异步运行时 | `tokio`（multi-thread） | 标准 |
| Async trait | `async-trait` | AsrProvider/AsrSession 用 |
| 日志 | `tracing` + `tracing-subscriber` | TUI/守护两种 sink；同时落 JSONL |
| 错误 | `thiserror`（库错误）+ `anyhow`（main） | 标准组合 |
| 配置校验 | `serde` 自带 + 自定义 `validate()` | 不引 garde 减少依赖 |
| 无锁注册表快照 | `arc-swap` | suppress 实现关键，也用于 StateStore 快照 + i18n 字典 |
| 取消令牌 | `tokio-util` 的 `CancellationToken` | 跟 tokio 配套 |
| State 序列化 | `serde_json` | UDS 协议 + history JSONL |
| 时间戳 | `time` | history/log 用 RFC3339；比 chrono 轻 |
| UDS | `tokio::net::UnixListener` / `UnixStream` | 标准库即可，无额外 crate |
| 本地 Whisper（M8） | `whisper-rs`（whisper.cpp 绑定，feature flag 控制） | 离线 ASR 备选 |
| LLM 后处理（M7） | `reqwest` + 手写 client | 简单 OpenAI 兼容 schema，不引 sdk |
| launchd plist 生成 | 模板字符串即可，不引依赖 | 简单 |

---

## 4. 目录结构（初稿）

```
shuohua/
├── Cargo.toml
├── README.md
├── REQUIREMENTS.md
├── docs/
│   ├── DESIGN.md
│   ├── SCHEMA.md
│   └── CLI.md
├── src/
│   ├── main.rs                 # CLI 入口（clap 子命令分发 → daemon / TUI / install / doctor 等）
│   ├── config.rs               # serde TOML + 热键字符串解析
│   ├── doctor.rs               # 终端识别 + AX/麦克风权限检查
│   ├── hotkey/
│   │   ├── mod.rs              # Combo / Modifier / KeyCode 类型
│   │   ├── tracker.rs          # 纯函数式状态机 + proptest
│   │   ├── registry.rs         # combo→handler 映射
│   │   └── provider_darwin.rs  # CGEventTap + pipe 桥
│   ├── voice/
│   │   ├── mod.rs              # 状态机 enum + main loop
│   │   ├── recorder.rs         # cpal 16kHz s16le mono + PcmConsumer
│   │   ├── vad.rs              # 客户端 VAD（WebRTC 包装 + 滑动窗口 + 切换防抖）
│   │   ├── finish.rs           # 收尾链
│   │   └── dispatch.rs         # 剪贴板 + Cmd+V
│   ├── asr/
│   │   ├── mod.rs              # AsrProvider / AsrSession trait
│   │   ├── types.rs            # Caps / SessionCtx / LanguageMode / AsrEvent / Boost
│   │   └── providers/
│   │       ├── doubao.rs       # M2 默认
│   │       ├── whisper_cpp.rs  # M8 离线
│   │       └── apple_speech.rs # M9 评估
│   ├── post/
│   │   ├── mod.rs              # PostProcessor trait + run_chain
│   │   ├── filler.rs           # M2.5 规则去口语词
│   │   ├── llm.rs              # M7 LLM 清洗
│   │   └── app_context.rs      # NSWorkspace.frontmostApplication
│   ├── state/
│   │   ├── mod.rs              # 内存状态机 + tokio::sync::broadcast 给订阅者
│   │   └── history.rs          # append-only history.jsonl + 派生统计
│   ├── ipc/
│   │   ├── mod.rs              # UDS server (daemon) / client (tui)
│   │   └── protocol.rs         # 命令/事件 serde 类型
│   ├── overlay/
│   │   ├── mod.rs              # 后台线程一侧：构造 OverlayCmd 推到主线程 channel
│   │   ├── view.rs             # AppKit 视图层（NSPanel + NSGlassEffectView + 子视图）
│   │   └── animations.rs       # CABasicAnimation / CATransition 包装
│   ├── autotype_darwin.rs      # CGEventPost Cmd+V
│   ├── clipboard_darwin.rs     # NSPasteboard
│   ├── cli/
│   │   ├── mod.rs              # clap derive，子命令分发
│   │   ├── doctor.rs           # shuo doctor（包含配置 validate + 打印 effective config）
│   │   ├── service.rs          # install / uninstall / start / stop / restart / status
│   │   └── smart.rs            # 裸跑 shuo：连 UDS or 起 daemon + TUI
│   ├── i18n/
│   │   └── mod.rs              # t!() 宏 + 字典加载，~100 行
│   └── tui/
│       ├── mod.rs              # ratatui 主循环（订阅 UDS）
│       ├── panes.rs            # 状态 / 历史 / pipeline 流水线视图
│       └── keybindings.rs
├── assets/
│   └── i18n/
│       ├── zh-CN.toml          # 默认中文
│       └── en-US.toml          # 默认英文
└── build.rs                    # 链接 AppKit / AudioToolbox / ApplicationServices；嵌入 i18n toml
```

---

## 5. 不变量与历史教训（必读）

照搬 Go 版 `ARCHITECTURE.md §12` 的核心条目，全部沿用：

1. CGEventTap 回调在专用 OS 线程上跑 CFRunLoop（Rust 等价：`std::thread::spawn` 不让出）
2. C → Rust 事件桥用 pipe，**不**用 cgo callback
3. 录音停止必须 drain residual + `stop_delay_ms`（默认 800ms），否则尾字会被切
4. `notify` 监听**配置目录**，不监听文件本身（编辑器换 inode）
5. `NSGlassEffectView` 必须作为子视图，**不**作 contentView，否则 AppKit 加 legibility blur
6. 普通字符键禁止做语音热键（`KeyCode::is_text_key()`），TUI 保存时回退
7. 热键注册必须在系统启动前完成；运行时新增（transient `Esc`/`R`）允许，但要保证 dispatcher 已起来
8. **热键 down/up 配对吞**：suppress 模式下，若 keydown 被吞，对应的 keyup 必须也吞，否则前台 App 看到孤立 keyup → modifier 状态泄漏。CGEventTap 回调里维护"已吞的物理键集合"，keyup 来时查表
9. AppKit 主线程与 tokio runtime 通过 `tokio::sync::mpsc` 通信。绝不能在 AppKit callback 里 block tokio future（用 `try_recv` 或 `dispatch_async` 到主线程）
10. `NSWorkspace.frontmostApplication` 必须在 toggle OFF 瞬间取一次缓存，**不**在 PostProcessor 内反复取——pipeline 跑期间用户可能切走，会拿到错的 app
11. **Stale UDS socket 清理**：daemon 崩了 socket 文件不自动删。两条路径必须处理：
    - daemon 启动 bind：bind 失败时尝试 `connect()`；连得通 = 已有 daemon 在跑，本进程退出；连不通 = stale socket → `unlink()` 后重新 bind
    - `shuo` 智能 fallback connect：`ECONNREFUSED` / `ENOENT` 都视作 daemon 不在，进入 fork 路径前 `unlink()` 一次
12. **Voice::Idle 时不持 cpal stream**：满足 <0.5% 空闲 CPU + 避免 always-recording 隐私问题。F9 触发时 ~50ms 启动延迟可接受（用户反应时间 200ms+ 覆盖）

---

## 6. 测试策略

**测试边界**：把纯函数和 I/O 边界严格分开。

| 模块 | 单测 | property test | 集成测试 |
|---|---|---|---|
| `config` parse / validate | ✓ | — | — |
| `hotkey::tracker` 状态机 | ✓ | ✓（key down/up 配对、suppress 不变量） | — |
| `voice` 状态机 | ✓（fake AsrProvider + fake recorder） | — | — |
| `vad`（WebRTC 包装 + 抖动滤波） | ✓（golden PCM 文件） | — | — |
| `post::chain` | ✓（fake processors，验证失败/超时/skipped） | — | — |
| `asr::providers::doubao` | — | — | ✓（命中真实 ASR，CI 跑要钥匙） |
| `ipc` UDS protocol | ✓（serde round-trip） | — | ✓（spawn daemon + 假 TUI） |
| `overlay` AppKit | — | — | 手测（M3 验收） |

**Fake / mock 边界**：
- `FakeAsrProvider`：根据 PCM 时长产 partial → segment → done，可注入 error。给 voice 状态机单测用。
- `FakePasteboard`：实现 `Clipboard` trait，记录写入。给 dispatch 单测用。
- `FakeHotkeyProvider`：给 voice 状态机喂 toggle/cancel 事件。
- CGEventTap pipe 桥：用 `os_pipe` 创建一对 fd，单测里写假事件，验证 tracker 状态。

**property test 重点**：
- KeyDown/KeyUp 配对：任意序列后，"已按下集合"大小恒定 ≥ 0
- Suppress：任何 keydown 被吞，对应 keyup 也被吞（§5 不变量 8）

---

## 7. 安全与隐私

- **配置文件权限**：首次写入 `~/.config/shuohua/config.toml` 时强制 `chmod 0600`。其他 toml 不强制（用户自己负责）。
- **API key 存储**：明文 TOML 是 v1 选择（仓库放模板，用户填）。未来可选 `keychain://` 前缀走 macOS Keychain，但 v1 不做。LLM provider 可写 `api_key_env = "ANTHROPIC_API_KEY"` 走环境变量。
- **日志**：`tracing` 默认级别 INFO，**不打识别文本**（避免 log.jsonl 里漏出敏感语音内容）。`--debug` 或专门 target 才打。
- **history.jsonl 明文**：是设计选择（用户唯一数据源），用户应当知道并定期清理。提供 `shuo doctor` 提示"history 已 X MB / Y 条"，但 v1 不自动清理。
- **不记录原始音频**（当前默认）：避免占用 + 隐私风险。M2 决定是否加 `--record-audio` 开关。
- **PostProcessor 隔离**：LLM processor 把识别文本发给第三方 API。doctor 启动时 warn 一次"启用 LLM 后处理意味着文本会发给 {provider}"。
