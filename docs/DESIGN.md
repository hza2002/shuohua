# 技术设计

外观规范 + 关键设计决策 + 技术选型 + 目录结构 + 不变量 + 测试 + 安全。
持久化/线协议格式见 [SCHEMA.md](./SCHEMA.md)，CLI 见 [CLI.md](./CLI.md)，阶段历史见 [CHANGELOG.md](../CHANGELOG.md)。

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
- 显示位置默认锚定 focused window 内部；配置只控制垂直位置
  `top | middle | bottom`，水平方向始终居中。
- macOS 26 不可用时静默回退到 `NSVisualEffectMaterialHUDWindow`（不弹错误）

#### 两排布局（v1）

```
╭───────────────────────────────────────────────────────────╮
│ ● Recording · 3.2s · 84字          Xcode  ·  filler→llm   │  ← 第 1 排 状态条
│ 今天我想写一篇关于分布式系统的一致性算法的文章｜             │  ← 第 2 排 ASR 实时文字
╰───────────────────────────────────────────────────────────╯
   ↑ Liquid Glass variant=19
   ↑ ~540×86 起，ASR 文本最多 5 行自适应增高
```

第 1 排（左→右）：
- **状态点**：`Idle=灰` / `Connecting=橙` / `Active=红` / `Idle子状态(思考中)=蓝` / `Stopping=黄` / `Error=红闪`
- **状态文字**：跟点同步切换
- **时长**（mm:ss 或 N.Ns）+ **当前字数**
- 右对齐：**当前 App 名称** · **当前 chain summary**（比如 `filler→llm`）

第 2 排：
- 当前 partial 文本（实时变化）
- 已 definite 的 segments 拼在 partial 前面，淡色显示
- 文本超过最大行数时保留尾部，前部用省略号截断
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
    SetState  { state: OverlayState },                            // 状态字 + icon + 颜色
    SetStats  { dur_ms: u64, chars: u32 },
    SetApp    { bundle_id, app_name, chain_summary },
    SetText   { text: String, kind: TextKind },                   // Partial / Final / Error
    AppendSegment { text: String },
    Notice    { text: String, ttl_ms: u32 },                      // meta 行临时黄字
    Hide,                                                          // notice 活着时延期到 ttl 到期再隐藏
    Dismiss,                                                       // ESC 专用：跳过延期立即关
    ReloadConfig { cfg: OverlayCfg },
    Relabel,                                                       // i18n 热切语言后让 view 重新翻译当前 label
}
```

**Overlay 反馈通道**（M7 重构后）：不开第二个 NSPanel，两条通道全部复用主 panel：

- **Notice**：`OverlayCmd::Notice { text, ttl_ms }` → meta 行（panel 右上角，平时显示 `app · chain`）临时换成黄字 warn 文案，TTL 到点自动恢复 `chain_summary`。用于非阻断 warn（PostProcessor step 失败 / 超时跳过）。默认 `NOTICE_TTL_MS = 3000`。
- **Error**：`OverlayCmd::SetText { kind: TextKind::Error, text }` → text 区（第 2 排，平时显示 partial/final）换成红字错误文案，盖住 partial/final（`display_text()` 优先级 error > final > segments+partial）。`SetText{Error}` 也驱动 view 内部的 `error_until` 倒计时，到点自动 hide。`ERROR_TTL_MS = 5000`，比 notice 长是因为错误文案需要用户读完决定要不要重试。
- **延期 Hide**：当成功路径发 `Hide` 时若 notice 还活着，view 设 `pending_hide=true`，等 notice 到期 tick 再真正隐藏；新 session 的 `SetState{Connecting}` 抢断 lingering（清 notice/error/pending_hide）；ESC 走 `Dismiss` 强制立刻关。

这个设计避免了"warn 一闪就被 dispatch 后的 Hide 吞掉"的问题，也让 Error 不被自动粘贴流程截胡。

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
| launchd 重定向的 stderr 文件 | release binary 的错误兜底日志 | 见 §2.13，不引入独立 log 框架；TUI **不**读这个文件 |

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

### 2.4 真正实现热键 Suppress + 完整 hotkey 语法

CGEventTap 装在 `CGEventTapOptions::Default` 模式下，回调读 `Suppressor` 决定 `CallbackResult::Drop` 还是 `Keep` —— `Drop` 在 `core-graphics ≥ 0.25` 的 safe wrapper 里真返回 NULL 给系统，事件就不会到前台 App。Event mask 含 `KeyDown` / `KeyUp` / `FlagsChanged`，回调将每个事件编码成 4 字节 `RawEvent` 写到 pipe 给 tokio 端 `Tracker`，即使被 suppress 的事件也照样写（Tracker 仍需要看见每个事件做 combo 匹配 / tap 检测）。

> **依赖红线**：suppress 落地依赖 `core-graphics ≥ 0.25` 的 `CallbackResult` API。0.24 的 safe wrapper 没有返 NULL 的路径，回滚版本 = suppress 静默失效。

#### Hotkey 配置语法

参考 VSCode `+` 分隔 + Karabiner 显式 `left_` / `right_` 前缀 + 自创 `:double` 后缀，组合成 single-line TOML 友好的 grammar：

```text
trigger     := combo (":double")?
combo       := token ("+" token)*
token       := modifier | key
modifier    := ("left_" | "right_")? mod_name
mod_name    := "cmd" | "command"                  // 都接受，canonical 是 "cmd"
            |  "ctrl" | "control"                  // 都接受，canonical 是 "ctrl"
            |  "opt"  | "alt"     | "option"       // 都接受，canonical 是 "opt"
            |  "shift"
key         := f1..f20 | a..z | 0..9
            |  space | tab | escape | return | delete | backspace
            |  up | down | left | right  (arrow keys; `left_cmd` 等模糊歧义靠下划线区分)
            |  ";" | "," | "." | "/" | "\" | "[" | "]" | "'" | "`" | "-" | "="
```

| 触发形态 | 语法示例 | 触发时机 | Suppress |
|---|---|---|---|
| 纯按键 | `f16` / `escape` / `a` | KeyDown 时（mods 必须**完全无**），auto-repeat 不重复触发 | 该 key 的 down + 配对 up |
| 修饰键 + 键 | `cmd+r` / `left_cmd+shift+r` / `cmd+;` | KeyDown 时 mods **精确匹配**（指定的必须按下、未指定的必须松开） | 仅 key 部分的 down/up；modifier 事件全放行 |
| 修饰键单按 | `right_shift` / `cmd` / `cmd+shift`（多修饰键也可） | "clean tap"：required mods 按下到松开期间无中间普通键 + 无额外修饰键 + 时长 < 500ms | 不吞任何事件（modifier 太常用） |
| 双击 | 上面任一种 + `:double` 后缀 | 两次 tap 在 400ms 内连发 | 同对应基础类型 |

**关键规则**：
- 全小写，大小写不敏感（normalize 到小写后处理）
- `left_` / `right_` 前缀只对修饰键有意义，未指定 = 任一侧匹配
- 修饰键有别名（`command` = `cmd`，`control` = `ctrl`，`alt` / `option` = `opt`），输入接受所有别名，`Display` 输出 canonical 3 字母形式以便 TUI capture round-trip 稳定
- 精确匹配：trigger 里没写的 modifier 必须松开。`cmd+r` 配置下按 `cmd+shift+r` 不触发（保持跟 VSCode 一致）
- `cmd+left_cmd` 退化为 `left_cmd`；`left_cmd+right_cmd`（两侧都按）M6 拒绝以避免 `ModMatcher` 类型膨胀，需要时再加 `BothSides` 变体
- `:double` 后缀仅一个，必须在末尾
- arrow keys `left`/`right`/`up`/`down` 跟修饰键 `left_cmd` 等通过下划线区分，token 化无歧义

**时间常数**（写死 `src/hotkey/tracker.rs` 顶部，不暴露给用户）：
- `MOD_HOLD_THRESHOLD = 500ms`：modifier-only tap 的"按下 → 松开"上限。超出视为长按而非 tap。500ms 是 BetterTouchTool / Hammerspoon 社区收敛值（Karabiner 默认 1000ms 偏慢，250ms 偏激进）
- `DOUBLE_TAP_WINDOW = 400ms`：双击两次之间上限。macOS Dictation Right Shift x2 实测 ~350ms，留 50ms 容错

**单按 tap 不会被双击 trigger 延迟**：每个 trigger 只有一种解释（单按 OR 双击），单按版本不需要等 400ms 窗口过期才 fire。

**模块拆分**（见 [MODULES.md](MODULES.md)）：
- `combo.rs`：`Combo` / `ModMatcher` / `ModMask` / `Side` / `ModType` 数据类型 + 精确匹配函数
- `parse.rs`：grammar → `Combo`
- `tracker.rs`：纯函数状态机 `RawEvent + Instant → HotkeyEvent`，分发到三个 sub-machine（纯键 / combo / modifier-only）+ 双击窗口
- `suppressor.rs`：纯函数 `RawEvent → bool`，按 trigger 类型决定吞什么
- `provider_darwin.rs`：CGEventTap + `decode_mods` 把 `CGEventFlags` 里 `NX_DEVICE*` 位转 `ModMask`

> **`ModMask` 设计**：8-bit 紧凑 packed mask，L/R per modifier。pipe 线协议 4 字节 `[kind, code_lo, code_hi, mods]`。`Instant` 不上线协议（Tracker 收到事件时 `Instant::now()`，亚毫秒延迟远低于 250/400ms 窗口）。
>
> **当前 Suppressor 注册表**：单 `Combo` 包在 `std::sync::Mutex<Suppressor>` 里跟主循环共享。CGEventTap callback 频率远低于 Mutex 竞争阈值。未来若加多 trigger 多绑定，可换 `ArcSwap` 做无锁快照。

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
- provider 私有配置类型化分离在自己的 TOML 文件，voice 模块永远不见
- provider 必须保证：`send_pcm(is_last=true)` 之后**至少**会出一个 `AsrEvent::Segment`，然后 `AsrEvent::Done`
- **音频 codec 在 provider 实现里写死**，不暴露给用户。codec 是工程权衡（CPU/带宽/server 兼容性），由 provider 作者拍板而非配置项

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
    pub hotwords: Vec<String>,        // 纯字符串数组；provider 自由解释（Doubao 直接塞、Apple 填 contextualStrings 等）
}

pub enum LanguageMode {
    Single(String),                       // "zh-CN" / "en-US"
    Multilingual { hint: Vec<String> },   // 中英混合走这个
}

#[async_trait]
pub trait AsrSession: Send {
    /// 喂 PCM (16kHz s16le mono)。is_last=true 表示后面没了，provider 必须在收到后吐 Segment + Done。
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<()>;

    async fn close(self: Box<Self>) -> Result<()>;
}

/// 单事件流。voice 模块 select 这根 channel 就够了。
pub enum AsrEvent {
    Partial { text: String, seq: u64 },                                 // 当前 utterance 最新猜测全文
    Segment { text: String, started_at: Instant, ended_at: Instant },   // 句末（server VAD 或 is_last 后）
    Error   { err: AsrError },                                          // 不要混进 Result，让 voice 决定降级
    Done,                                                               // session 终结（is_last + 最后一段已发完）
}

/// 用 thiserror 结构化，M3+ overlay 直接 match err.kind() 分发样式，零字符串解析。
#[derive(thiserror::Error, Debug)]
pub enum AsrError {
    #[error("auth failed: {0}")]      Auth(String),       // 401/403：检查 key
    #[error("network: {0}")]          Network(String),    // dial / send / recv 通信失败
    #[error("quota exceeded")]        Quota,              // 429 / server 返回 quota error
    #[error("protocol: {0}")]         Protocol(String),   // 帧解码失败、payload 字段缺失
    #[error("timeout waiting final")] Timeout,            // is_last 后等 Done 超时
    #[error("server: {0}")]           Server(String),     // server 返回 error frame (Doubao msg_type=0b1111)
    #[error("canceled")]              Canceled,           // 用户取消；voice 模块静默处理，不报错
}
```

**为什么删 `server_side_vad` cap**：原本用来告诉 voice "session 中间会不会冒 Segment"。改用单事件流后，voice 行为统一（来什么处理什么），不需要分支。Provider 实现保证 `is_last=true` 后至少出一个 Segment 即可。

**为什么 hotwords 是 `Vec<String>` 而不是结构化 `{ word, boost }`**：现有 provider 没有一家支持 per-word boost（Doubao 接词列表、Apple SpeechAnalyzer 用 contextualStrings）。统一结构是为假想未来设计、YAGNI。真接入支持 boost 的 provider 时，它在自己的 `asr/<provider>.toml` 里定义私有字段，跟主 hotwords 列表互不影响。

**错误处理策略**：
- **M2 不自动重试**。用户操作可见可重复（再按一次 F16），自动重试反而隐藏失败、增加调试难度
- **dispatch 只在 `Final`/`Segment` 拼完才写剪贴板**，没收到末段就不上屏（部分识别上屏是 bug）
- **Stopping 等 final 超时 5s**（M2 单 session 场景；正常 final 应在 send last 后 < 1s）
- **`AsrError::Canceled` 静默处理**，voice 模块不报 stderr、不发 error overlay

#### 各 provider 怎么映射

| Provider | Partial | Segment | 备注 |
|---|---|---|---|
| Apple SpeechAnalyzer (macOS 26) | `isFinal=false` → `AsrEvent::Partial` | `isFinal=true` → `AsrEvent::Segment` | 本地优先；Swift helper 桥接 Swift-only API；provider 内部把 canonical i16 PCM 转 AVAudioPCMBuffer |
| Doubao SAUC | `definite=false` → `AsrEvent::Partial` | `definite=true` → `AsrEvent::Segment` | 可用云端 provider；codec 写死 raw PCM |
| 纯 batch ASR API | 无 | 无 | **不入选** |

#### 配置文件布局

```
~/.config/shuohua/
├── config.toml              # 全局：hotkey / voice / post timeout
├── apps/
│   ├── default.toml         # 默认 app profile：选择 ASR + post chain + 覆盖项
│   └── com.mitchellh.ghostty.toml
├── post/
│   ├── rules/
│   │   └── filler.toml      # 后处理组件定义：规则
│   └── llm/
│       └── deepseek.toml    # 后处理组件定义：LLM provider/model/prompt 默认值
└── asr/
    ├── doubao.toml          # ← 文件名 == provider 名
    └── apple.toml           # 可省略；无 secret，缺省值可直接工作
```

`config.toml` 只保留全局行为开关；apps/post 目录固定走 XDG 路径：

```toml
[hotkey]
trigger = "f16"

[voice]
stop_delay_ms = 800
record_audio  = false                        # ← 见 §7
vad_trace     = false                        # dev-only；需 feature=dev-vad-trace，见 SCHEMA §4

[voice.vad]
backend = "silero"
threshold = 0.5
pause_silence_ms = 1500
pre_roll_ms = 300
max_overlap_ms = 200
min_start_voiced_frames = 2

[post]
timeout_ms = 2000
```

`apps/default.toml`（app profile，负责组合；app 相关热词和局部覆盖优先放这里）：

```toml
name = "Default"

[asr]
provider = "apple"                          # → 加载 asr/apple.toml
hotwords = ["Rust", "tokio", "Kubernetes"]   # provider 自由解释；不支持的 provider 静默忽略

[post]
chain = ["rule:filler", "llm:deepseek"]
```

`asr/doubao.toml`（provider 私有，voice 模块永远不见）：

```toml
app_key     = ""
access_key  = ""
resource_id = "volc.bigasr.sauc.duration"
language    = "zh-CN"         # 可省略；省略时自动中英混合
enable_itn  = true
enable_punc = true
enable_ddc  = true
stream_mode = 2               # 可选实验字段；不确定时注释掉
ai_vad      = true            # 可选实验字段；不确定时注释掉
```

`asr/apple.toml`（provider 私有；文件可省略）：

```toml
language       = "zh-CN"  # 可省略；省略时从 SessionCtx 选 zh-CN 优先
install_assets = true     # 首次使用时允许系统下载/安装本地 SpeechAnalyzer 资产
```

AppleProvider 保持 `AsrSession::send_pcm(&[i16])` 的 canonical 输入不变。SpeechAnalyzer 在 macOS 26 上通常返回 `16kHz mono int16` 或 `float32` 兼容格式；具体 `AVAudioPCMBuffer` 适配发生在 provider 的 Swift helper 内部，Recorder 不感知 provider 私有音频格式。

provider 之间**完全不共享 schema**。每个 provider impl 自己 deserialize 自家文件。hotwords 由 app profile 选择后塞进 `SessionCtx`。

配置分层原则：ASR / post component 文件描述可复用能力和默认参数；app profile 描述“这个 App 选哪套 ASR、哪些 hotwords、跑哪条 post chain，以及哪些 provider 字段要浅覆盖”。当某个 prompt、hotwords 或 ASR 旋钮明显只服务一个 App 时，归属 app profile override；当它代表一个可复用默认能力时，归属 `asr/*.toml`、`post/llm/*.toml` 或 `post/rules/*.toml`。数组/字符串/表字段都是写了就替换，不做深层智能合并。

### 2.9 客户端 VAD + 多段 session（"思考不计费"机制）

> **状态**：M10 设计中，当前默认运行路径仍是单 ASR session。M2.5 试过 webrtc-vad，在真实声学环境里误判率高（风扇/空调嗡鸣谐波会过频域检测、RMS 门无法同时满足灵敏度与稳定性），不适合生产。
> M10 采用 Silero VAD shadow trace 验证后再正式启用，详细控制协议见 [M10](M10.md)。

#### Doubao 计费模型（核实后的事实）

Resource ID `volc.bigasr.sauc.duration` 的 `duration` 是关键：**按音频时长计费**（约 3.5 元/小时，0–300h 阶梯）。默认免费 10 路并发，单用户单连接不会触发限额。

**省的不是"会话数"，而是静默时间不喂音频给 Doubao**。10 分钟沉默一直喂静音 PCM 扣 10 分钟时长费；client VAD 检测到静音不喂 = 0 费。

#### 两种实现方案

| 方案 | 做法 | 优点 | 缺点 |
|---|---|---|---|
| **A. 关 session**（v1 选） | 静音 ≥3s → close → 静音期不连 → 恢复说话开新 session | 简单，Doubao 行为确定 | 重连首字延迟 ~300–500ms |
| **B. 保 session 暂停喂** | 静音后不发 PCM（连接保持）→ 恢复直接喂 | 首字延迟 ~200ms | 需心跳防超时 |

**v1 = A**。M3 之后实测首字延迟刺眼再切 B。

#### Voice 状态机支持（§2.1 Recording 含 Active/Idle 子状态）

| 当前 | 事件 | 下一个 |
|---|---|---|
| `Recording.Active` | VAD 判定静音 ≥ pause_asr_silence_ms | `Recording.Idle`（关 ASR，把 final 追加到 pending_output） |
| `Recording.Idle` | VAD 判定有声 | `Recording.Active`（开新 session；dump pre-roll 再喂当前帧） |
| `Recording.*` | 静音 ≥ auto_stop_silence_ms | `Stopping`（防忘按 toggle） |
| `Recording.*` | toggle OFF / cancel / 错误 | `Stopping` |

v1 不实现此子状态机（Recording 始终 Active），Voice enum 也不在代码中显式存在（M3 再抽）。

#### 未来配置文件占位

以下字段已在 `config.toml` 设计里预留但 v1 不加载（serde `#[serde(default)]` 无对应字段 → 无需配置）：

```toml
[voice]
pause_asr_silence_ms = 3000      # 静音多久后关 ASR session（未来）
auto_stop_silence_ms = 600000    # 静音多久后完全停止录音（未来）
```

#### 关键不变量

1. **VAD 独立于 provider**：任何 provider 不需要关心 VAD。
2. **段间无分隔符**：provider 保证 emit 的 segment 直接 concat 就是它想要的最终文本。Doubao 自带句末标点；其他 provider 若不自带，应在 adapter 内部补，不暴露 voice 层旋钮。
3. **cpal 跟随 Voice 状态机**：Voice::Idle 时绝不持有 cpal stream。

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
            Ok(Err(e)) => PipelineStep::err(p.name(), started.elapsed(), e.to_string()),
            Err(_)     => PipelineStep::timeout(p.name(), started.elapsed()),
        };
        steps.push(step);
    }
    (current, steps)
}
// caller（finish.rs）拿到 steps 后再统一对 Error/Timeout 推
// OverlayCmd::Notice，把"什么 step 失败"的 UI 决策跟 chain 执行解耦。

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
- **失败/超时都推 notice**：caller 遍历 steps，对每个非 Ok/Skipped 状态发 `OverlayCmd::Notice { text, ttl_ms }`，meta 行黄字 3s。Hide 看到 notice 活着会自动延期，避免 dispatch 后 hide 一次性把 warn 吞掉
- 所有失败 + 时延都写进 `pipeline[]`，进 history.jsonl + UDS 事件

#### App Profile 与 Post Components

```
~/.config/shuohua/
├── config.toml                          # 全局，只指路
├── apps/
│   ├── default.toml                     # 默认 profile
│   ├── com.apple.dt.Xcode.toml          # Xcode 专属 profile
│   └── com.mitchellh.ghostty.toml
└── post/
    ├── rules/
    │   └── filler.toml
    ├── llm/
    │   └── deepseek.toml
    └── scripts/                         # 预留；M7 不执行用户脚本
```

匹配逻辑：toggle ON 时取一次 `frontmost_bundle_id`，去 `apps/<bundle_id>.toml` 找；找到就用，找不到 fall back 到 `apps/default.toml`。该 profile 决定本次录音的 ASR provider、hotwords、provider 覆盖项和 post chain。toggle OFF 时只再取一次 AppContext 作为 prompt 变量，**不重新选择 profile**，避免录音中切 App 导致 ASR/post 配置中途变化。

目录固定：

- app profile：`~/.config/shuohua/apps/default.toml` / `~/.config/shuohua/apps/<bundle_id>.toml`
- post components：`~/.config/shuohua/post/rules/*.toml` / `~/.config/shuohua/post/llm/*.toml`

主 `config.toml` 只配置单步超时：

```toml
[post]
timeout_ms = 2000     # 单步 processor 超时
```

app profile 长这样：

```toml
# apps/com.mitchellh.ghostty.toml
name = "Ghostty"

[asr]
provider = "doubao"
hotwords = ["Rust", "tokio", "Kubernetes", "cargo", "git", "zsh"]

[post]
chain = ["rule:filler", "llm:deepseek"]

[post.llm.deepseek]
model = "deepseek-v4-flash"
system_prompt = "你是终端语音输入清洗器，只输出清洗后的文本。"
```

Post component 长这样：

```toml
# post/rules/filler.toml
type     = "rule"
patterns = ["嗯", "啊", "呃", "那个", "就是"]
```

```toml
# post/llm/deepseek.toml
type        = "llm"
format      = "openai"             # anthropic | openai
name        = "deepseek"           # provider display/default routing name
base_url    = "https://api.deepseek.com"
model       = "deepseek-chat"
api_key     = "sk-..."
system_prompt = "你是语音输入文本清洗器，只输出清洗后的文本。"
prompt = """
当前 App: {{app_name}} ({{bundle_id}})
原始文本: {{text}}
清洗语音识别文本。
只输出清洗后的文本，不要解释。
"""

[extra_body]                         # provider-specific OpenAI-compatible 请求体扩展
thinking = { type = "disabled" }     # DeepSeek 专属；不是 OpenAI 通用默认字段
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
- **Overlay 只显示最终上屏文本**（chain 最后一项的 text；chain 空则 = raw），单步失败/超时通过 meta 行 notice 通知；致命错误（ASR 中断 / 剪贴板失败 / mic watchdog）通过 text 区 error 红字反馈，并跳过 dispatch。

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
state_thinking   = "思考中"
state_stopping   = "收尾"
state_error      = "错误"
word_count       = "{n}字"

[notice]                                  # meta 行 warn（非阻断）
step_timeout     = "{name} 超时，已跳过"   # 支持 {var} 占位符
step_failed      = "{name} 失败，已跳过"

[error]                                   # text 区致命错误（5s 自动 hide / ESC 立即关）
no_audio         = "没有收到麦克风音频，请检查输入设备"
recorder_start   = "录音启动失败"
asr_open         = "ASR 连接失败"
asr_runtime      = "ASR 中断，本次录音未保存到剪贴板"
dispatch         = "粘贴失败，可在历史里找回"
```

```toml
# assets/i18n/en-US.toml
[overlay]
state_idle       = "Idle"
state_recording  = "Recording"
state_connecting = "Connecting"
state_thinking   = "Thinking"
state_stopping   = "Stopping"
state_error      = "Error"
word_count       = "{n} words"

[notice]
step_timeout     = "{name} timed out, skipped"
step_failed      = "{name} failed, skipped"

[error]
no_audio         = "No microphone audio — check input device"
recorder_start   = "Failed to start recording"
asr_open         = "ASR connection failed"
asr_runtime      = "ASR interrupted — nothing pasted"
dispatch         = "Paste failed — text saved in history"
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
/// t!("notice.step_failed", name = "filler") → "filler failed, skipped"
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
| Overlay 文本（状态点 label、notice、error、字数单位） | ✓ |
| TUI 全部文本 | ✓ |
| Doctor 输出 | ✓ |
| `shuo help` / clap 自动文案 | 默认 en-US（clap 自带不易换；help 实际上是英文用户也能看懂的） |
| 日志（stderr） | en-US 固定（debug 用，不走 i18n） |
| history.jsonl | 不本地化（数据格式） |

#### 热重载

`ui.language` 改完保存即时生效（字典换 + overlay 当前 state label 重译）。机制走 [§2.12 reload 模块](#212-配置热重载reload-模块)，subscriber 是 `reload::spawn_i18n`。

---

### 2.12 配置热重载（reload 模块）

> M3.f 已实现完整机制；M5 收口只补 ⏸ 项。

#### 模块边界

独立文件 `src/reload.rs`，单向依赖 config / overlay / i18n / hotkey 暴露的对外 API（`OverlayHandle`、`i18n::init`、`hotkey::parse::parse`），**不被这些模块反向 import**。等于一个集中放置的"翻译层"——watcher 这边一个 source，subscriber 这边 N 个 sink。

```
reload.rs
├── pub fn watch(path) -> Result<Rx>           ── notify watcher（专用 std::thread）
├── pub fn spawn_overlay(rx, OverlayHandle)    ── [overlay] 段 diff → OverlayCmd::ReloadConfig
├── pub fn spawn_i18n(rx, OverlayHandle)       ── ui.language diff → i18n::init + OverlayCmd::Relabel
└── pub fn spawn_hotkey(rx, mpsc::Sender<Combo>) ── [hotkey].trigger diff → 主循环 swap Tracker + Suppressor
```

`Rx = tokio::sync::watch::Receiver<Arc<Config>>`，跟 [§2.5](#25-去掉-plugin-抽象) 里"配置变化通过 watch 广播"对上。

#### 实现要点

- **监听目录而非文件**（[§5 不变量 #4](#5-不变量与历史教训必读)）：编辑器保存常做 atomic rename（inode 替换），监听文件本身会丢事件
- **150ms debounce**：编辑器一次保存常触发 2-3 条事件，合并掉
- **parse 失败保留旧值**：只打日志 `[reload] parse failed, keeping previous: ...`，不让 watch::Sender 发空值
- **subscriber 自带 diff**：每个 subscriber 缓存 `prev`，只在自己关心的字段变化时才动作，避免无关字段保存导致的视觉抖动

#### 字段覆盖矩阵

| 字段 | 生效时机 | 走哪条路径 | 状态 |
|---|---|---|---|
| `[overlay].*` 所有字段 | 立即（next render） | `spawn_overlay` → `rebuild_chrome` | ✓ |
| `ui.language` | 立即（重译 state label） | `spawn_i18n` → `i18n::init` + `Relabel` | ✓ |
| `[hotkey].trigger` | 立即（下次按键判定） | `spawn_hotkey` → mpsc<Combo> → `tokio::select!` 换 Tracker + Suppressor | ✓ |
| `[voice].*` 全部 | 下次起 session | daemon 主循环 `cfg_rx.borrow()` 取最新快照 | ✓ |
| `apps/*.toml` 的 `[asr]` / `[post]` / override | 下次起 session | daemon 主循环按 toggle ON 的 App profile 选择 | ✓ |
| `post/rules/*.toml` / `post/llm/*.toml` | 下次起 session | app profile 的 `[post].chain` 引用组件 | ✓ |
| 手动触发 `{"op":"reload_config"}` | 立即 | 走 UDS server | ✓ |

#### Hotkey trigger 特别说明

`CGEventTap` 在 OS 层捕获所有键盘事件、不过滤——trigger 的切换只影响 `Tracker.on_raw()` 的判定。所以重置成本是"主循环里 `tokio::select` 收到新 keycode → `tracker = Tracker::new(new_code)`"，不需要拆 CGEventTap。`Tracker::new` 会把 `trigger_pressed` 归零，避免旧 trigger 半按状态串到新 trigger。

parse 失败（非法 trigger 字符串）只打日志保留旧 trigger，不向主循环发新 keycode。

#### M5 收口结果

- `shuo doctor` 已实现：打印 `effective config`，校验主 config、hotkey、默认麦克风输入、Doubao 配置、UDS 状态、launchd plist 和权限状态
- `shuo install/uninstall/start/stop/restart/status` 已实现：launchd plist 使用当前 `shuo` 绝对路径，状态优先走 UDS `daemon_status`
- `UDS {"op":"reload_config"}` 已接入：走 watcher 同一路径 parse + broadcast，不绕过 `watch::Sender`
- app profile 的 `asr` 已按配置在下一次录音开始时重建，不在录音中途热替换 session

---

### 2.13 日志门禁（release vs debug）

shuohua 是 launchd 后台 daemon。release binary 跑起来时 stderr 会被 launchd `.plist` 的 `StandardErrorPath` 重定向到日志文件，**长年累积**——所以发到 stderr 的每一行都按"将来谁会读这条"标准来判断。

#### 三层

| 层 | 角色 | 在 release | 实现 |
|---|---|---|---|
| **canonical 记录** | 已完成 session 的事实（raw_text、segment 时间戳、pipeline 步骤、status、error） | 是，写 `~/.local/state/shuohua/history.jsonl` | `state::history::append_default` |
| **stderr error 兜底** | 错误 / 警告 / 启动 OK / 致命路径 | 是，走 stderr → launchd 日志文件 | `eprintln!` 直调 |
| **stderr debug narration** | partial / segment / 探针 / 每帧细节 / 成功路径口播 | **否**（编译期消除） | `crate::debug_println!` 宏 |

核心原则：**history.jsonl 是真事实源**，stderr 只在 history 兜不到的失败路径（连接前崩、panic、未到 record state 的错误）当兜底。narration 是开发期工具，不该污染长驻 daemon 的日志文件。

#### 怎么判一条 `eprintln!` 该归哪

| 这条日志的内容是 | 归层 |
|---|---|
| 错误 / 异常 (`❌`)、警告 (`⚠`) | `eprintln!`（release） |
| 启动一次性信息（"daemon ready"、配置摘要、apps/post 目录）| `eprintln!`（release） |
| 单 session 内逐步 narration（"▶ recording"、"partial#N"、"segment"、"✓ 剪贴板已写入"）| `debug_println!` |
| 探针 / 内省（drift、glass probe、协议帧 dump）| `debug_println!`（或整个模块 `#[cfg(debug_assertions)]`）|
| history.jsonl 里**已经记录**的内容（pipeline 步骤耗时、最终文本、recording id）| `debug_println!` |
| **正常路径**的 OK 行（"reloaded"、"language → en"）| `debug_println!` |

#### 实现细节

- `src/log.rs` 定义 `debug_println!`：debug build 直接 `eprintln!`；release build 用 `let _ = format_args!(...)` 消耗参数（避免 unused_variables 警告）但不做 IO。
- 探针类对象（如 `DriftProbe`）走两份 `cfg` 分支的 struct + impl：debug build 持 `Vec<String>`、方法干活；release build zero-sized struct、方法空体。
- 表达式位置（如 match arm）能直接用：`Ok(()) => crate::debug_println!("✓ ...")`。

#### 不变量

1. **不引入 `tracing` / `log` 等框架**，直到 shuohua 真有第二个用户 / 需要 runtime 级别切换 / 需要 structured fields。现在的二级门禁够了。
2. **不在 release 路径里做 narration**。逐条决策点见上表，新写代码也按此分。
3. **history.jsonl schema 升级走 SCHEMA.md**，不依赖 stderr 推测过去发生了什么。

---

## 3. 技术选型

| 用途 | crate | 理由 |
|---|---|---|
| Objective-C 互操作 | `objc2` 0.6 + `objc2-app-kit` 0.3 + `objc2-foundation` 0.3 | 现役标准，活跃维护 |
| Core Graphics / CGEventTap | `core-graphics` (≥ 0.25), `core-foundation` | CGEventTap pipe 桥的基础。0.25 的 `CallbackResult::Drop` 是 suppress 落地依赖（见 §2.4） |
| 录音 | `cpal`（首选）/ `coreaudio-rs`（备选） | cpal 简单，coreaudio-rs 控制更细。先用 cpal |
| VAD（M10） | Silero VAD via optional `voice_activity_detector` / ORT | WebRTC/RMS 真实环境误判高；Silero 先以 dev trace 验证，正式接入见 [M10](M10.md) |
| PCM 通道（callback→consumer） | `tokio::sync::mpsc::unbounded` | M2 已验稳定；Go 版 syscall pipe 都稳跑，mpsc 更轻 |
| 唯一 ID | `ulid` | history record id；26 字符短于 UUID，含时序信息 |
| WebSocket | `tokio-tungstenite` + `native-tls` | tokio 生态首选；DoubaoProvider 用。macOS 原生 Security framework 走 native-tls（无 rustls CryptoProvider 配置负担、无 OpenSSL；跨平台时再切 rustls） |
| TUI | `ratatui` + `crossterm` | Bubble Tea 的事实替代；**唯一前台 UI** |
| TOML | `toml` + `serde` | 标准 |
| 文件监听 | `notify` | 监听**目录**而非文件（避免 inode 替换） |
| 异步运行时 | `tokio`（multi-thread） | 标准 |
| Async trait | `async-trait` | AsrProvider/AsrSession 用 |
| 日志 | std `eprintln!` + 自家 `debug_println!` 宏 | 不引日志框架（见 §2.13）；release 走 launchd stderr，debug build 看完整 narration |
| 错误 | `thiserror`（库错误）+ `anyhow`（main） | 标准组合 |
| 配置校验 | `serde` 自带 + 自定义 `validate()` | 不引 garde 减少依赖 |
| 无锁注册表快照 | `arc-swap` | suppress 实现关键，也用于 StateStore 快照 + i18n 字典 |
| 取消令牌 | `tokio-util` 的 `CancellationToken` | 跟 tokio 配套 |
| State 序列化 | `serde_json` | UDS 协议 + history JSONL |
| 时间戳 | `time` | history/log 用 RFC3339；比 chrono 轻 |
| UDS | `tokio::net::UnixListener` / `UnixStream` | 标准库即可，无额外 crate |
| Apple SpeechAnalyzer（M8） | Swift helper + `build.rs` 编译嵌入 | macOS 26 本地流式识别；隐私优先默认候选 |
| LLM 后处理（M7） | `reqwest` + 手写 client | 简单 OpenAI 兼容 schema，不引 sdk |
| launchd plist 生成 | 模板字符串即可，不引依赖 | 简单 |

---

## 4. 目录结构（初稿）

```
shuohua/
├── Cargo.toml
├── README.md
├── CHANGELOG.md
├── docs/
│   ├── DESIGN.md
│   ├── SCHEMA.md
│   └── CLI.md
├── src/
│   ├── main.rs                 # smart fallback；--daemon 跑 AppKit + tokio daemon；M5 再接 clap 子命令
│   ├── config.rs               # serde TOML + 热键字符串解析（纯类型，无 I/O）
│   ├── log.rs                  # debug_println! 宏 + 日志门禁原则（见 §2.13）
│   ├── reload.rs               # notify watcher + overlay/i18n/hotkey subscribers（M3.f）
│   ├── doctor.rs               # 终端识别 + AX/麦克风权限检查
│   ├── hotkey/
│   │   ├── mod.rs              # Combo / Modifier / KeyCode 类型
│   │   ├── tracker.rs          # 纯函数式状态机 + proptest
│   │   ├── registry.rs         # combo→handler 映射
│   │   └── provider_darwin.rs  # CGEventTap + pipe 桥
│   ├── voice/
│   │   ├── mod.rs              # 子模块声明
│   │   ├── recorder.rs         # cpal 16kHz s16le mono → mpsc
│   │   ├── finish.rs           # 录音生命周期 + filler pipeline + dispatch
│   │   └── dispatch.rs         # 剪贴板 + Cmd+V
│   ├── asr/
│   │   ├── mod.rs              # AsrProvider / AsrSession trait
│   │   ├── types.rs            # Caps / SessionCtx / LanguageMode / AsrEvent / Boost
│   │   └── providers/
│   │       ├── apple.rs              # Apple SpeechAnalyzer provider
│   │       ├── apple_helper.swift    # Swift-only SpeechAnalyzer bridge；build.rs 编译嵌入
│   │       └── doubao.rs             # Doubao SAUC provider
│   ├── post/
│   │   ├── mod.rs              # PostProcessor trait + PipelineText + run_chain（M2.5）
│   │   ├── filler.rs           # RuleBasedFiller（M2.5）
│   │   ├── llm.rs              # M7 LLM 清洗
│   │   └── app_context.rs      # NSWorkspace.frontmostApplication（M7）
│   ├── state/
│   │   ├── mod.rs              # 内存状态机 + tokio::sync::broadcast 给订阅者
│   │   └── history.rs          # append-only history.jsonl + 派生统计
│   ├── ipc/
│   │   ├── mod.rs              # 子模块声明 + 公共类型 re-export
│   │   ├── protocol.rs         # 命令/事件 serde 类型（M4，schema 见 SCHEMA.md §1）
│   │   ├── server.rs           # daemon 侧 UnixListener + 每 client 一个 tokio task；订阅 StateEvent broadcast
│   │   └── client.rs           # TUI / 外部脚本侧连接 + 帧解析
│   ├── overlay/
│   │   ├── mod.rs              # 后台线程一侧：构造 OverlayCmd 推到主线程 channel
│   │   ├── view.rs             # AppKit 视图层（NSPanel + NSGlassEffectView + 子视图）
│   │   └── animations.rs       # CABasicAnimation / CATransition 包装
│   ├── autotype_darwin.rs      # CGEventPost Cmd+V
│   ├── clipboard_darwin.rs     # NSPasteboard
│   ├── cli/                    # M5 引入
│   │   ├── mod.rs              # clap derive，子命令分发
│   │   ├── doctor.rs           # shuo doctor（包含配置 validate + 打印 effective config）
│   │   └── service.rs          # install / uninstall / start / stop / restart / status
│   ├── i18n/
│   │   └── mod.rs              # t!() 宏 + 字典加载，~100 行
│   └── tui/
│       ├── mod.rs              # ratatui 主循环（订阅 UDS）；Status / History / Settings 三页
│       ├── panes.rs            # 状态 / 历史 / pipeline / 配置浏览渲染
│       └── keybindings.rs      # Tab/Shift-Tab + 1/2/3 翻页；h/l 留给未来设置项调整
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
13. **麦克风可用性靠运行时 watchdog，不靠预检**：macOS 没有可靠的事前探测（`AppleClamshellState` + 设备名特判是过度脆弱的 workaround，合盖时 cpal 仍 callback 全 0 帧）。`recorder::start()` 同步段只校验 cpal build/play 失败；可用性的真相 = 主 select 里的 1s 首帧 watchdog：cpal stream play 成功后 `FIRST_AUDIO_TIMEOUT_MS = 1000ms` 内若所有 PCM 样本都 ≤ `MIN_NONZERO_AMPLITUDE = 8`（约 -72 dBFS，严格高于精确 0 silence，严格低于消费级 ADC 本底噪声），判定设备不可用并走 error overlay。阈值不进配置——超过这个时间几乎是设备问题，不是用户该调的旋钮
14. **Error 路径不上屏不写剪贴板**：录音 / ASR 中途崩溃（`terminal_error` 任意 setter）→ 跳过 post chain → 跳过 `dispatch::dispatch` → history 写 status=Error 含累积 segments。理由：半成品上屏会误导（用户以为成功），auto_paste 还可能粘到用户已切走的应用，silent 覆盖剪贴板更糟。需要回捞的用户从 TUI history 翻一行（j/k + Enter copy）。`auto_paste` 永远只在成功路径生效

---

## 6. 测试策略

**测试边界**：把纯函数和 I/O 边界严格分开。

| 模块 | 单测 | property test | 集成测试 |
|---|---|---|---|
| `config` parse / validate | ✓ | — | — |
| `hotkey::tracker` 状态机 | ✓ | ✓（key down/up 配对、suppress 不变量） | — |
| `voice` 状态机 | ✓（fake AsrProvider + fake recorder） | — | — |
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
- **API key 存储**：明文 TOML 是 v1 选择（仓库放模板，用户填）。未来可选 `keychain://` 前缀走 macOS Keychain，但 v1 不做。LLM post 配置用 `api_key = "..."`，不写入 history。
- **日志**：见 §2.13。release stderr 只兜底错误 / 警告 / 启动；debug build 才打 partial / segment 等识别文本细节。launchd 日志文件不含识别内容。
- **history.jsonl 明文**：是设计选择（用户唯一数据源），用户应当知道并定期清理。提供 `shuo doctor` 提示"history 已 X MB / Y 条"，但 v1 不自动清理。
- **音频留存可选**：`voice.record_audio = false` 是默认。开启时写 `~/.local/state/shuohua/audio/<recording_id>.wav`（跟 history.jsonl 同一根目录，state dir 语义 = "用户主动留档，不允许被系统 cache cleanup 清掉"；不用 `/tmp`/`~/.cache`）。文件名 = 该次 recording 的 ULID，跟 history.jsonl 行天然 join。多 session 情况（M2.5+）下整次录音仍是一个 wav（含静音停顿），段边界由 `history.asr.sessions[].started_at/ended_at` 切分。关闭路径完全跳过 wav 写入逻辑，零开销，零额外内存分配。
- **PostProcessor 隔离**：LLM processor 把识别文本发给第三方 API。doctor 启动时 warn 一次"启用 LLM 后处理意味着文本会发给 {provider}"。
