# shuohua（说话）需求文档

> **项目名**：shuohua（说话）—— "shuohua" 拼音表义最直白，与产品定位一致；汉字"说话"既是字面，又有古义 *说=yuè=愉悦*（《论语》"不亦说乎"）的双关
> **CLI 命令**：`shuo` —— 单字"说"，4 字母好打
> **crate 名**：`shuohua` —— 在 crates.io 注册的包名，binary 名通过 `[[bin]] name = "shuo"` 单独指定
>
> 状态：起草中。本文档是 Rust 重写的真实需求源头，所有后续设计、技术选型、里程碑都以此对齐。
> 上游参考：Go 版 `just-talk-go/docs/ARCHITECTURE.md`（包含历史踩过的坑与不变量），但本项目从零设计，不照搬。

## 深入阅读

| 文档 | 内容 |
|---|---|
| 本文档 | 定位 / 平台约束 / 功能列表 / 性能目标 / 决策 / 里程碑 |
| [docs/DESIGN.md](docs/DESIGN.md) | 技术设计：外观、状态机、ASR trait、VAD、PostProcessor、目录结构、不变量、测试、安全 |
| [docs/SCHEMA.md](docs/SCHEMA.md) | UDS 协议 + history.jsonl 字段定义 |
| [docs/CLI.md](docs/CLI.md) | CLI 命令 + launchd plist 模板 |
| [docs/MODULES.md](docs/MODULES.md) | 模块实现状态：M1 已实现的源码树 + 各里程碑规划的新增路径 |

---

## 1. 一句话定位

一个常驻在 macOS（26+ / Tahoe）的语音输入助手：全局热键启停麦克风，识别结果写剪贴板并可选模拟 `Cmd+V` 上屏。强调**稳定、低负载、现代外观、可配置**。

## 2. 平台与运行时硬约束

| 项 | 要求 |
|---|---|
| OS | macOS 26 (Tahoe) 及以上，仅支持 Apple Silicon + Intel 的 macOS |
| 不支持 | Linux / Windows / X11 / Wayland / 非 cgo 等价路径，统统不要 |
| 语言 | Rust（stable toolchain，cgo 等价是 `objc2`） |
| 权限 | Accessibility（CGEventTap）+ Microphone（AudioQueue/cpal）。继续走"授权给启动的终端 App"模式 |

## 3. 用户可见功能

### 3.1 核心功能（必须）

- 全局热键启停录音（toggle 模式；hold 模式经评估不做——dictation 场景按住几十秒不现实，靠 `auto_stop_silence_ms` 兜底忘按）
- 录音流 → ASR provider → 识别文本（首发支持火山豆包 bigmodel_async）
- **流式 partial 是 ASR 契约的必需部分**：任何 provider 必须能边录边给出实时增量假设
- **同一次 Recording 内的多段 ASR session**：客户端 VAD 静默时关 ASR 省计费，恢复时开新 session，跨段文本累积一次性上屏
- 段间分隔符可配置（默认空格）
- 识别结果写剪贴板，可选自动 `Cmd+V` 上屏
- 中英混合识别支持，热词列表（专门给英文技术词加 boost）
- 顶层悬浮状态胶囊（Liquid Glass）显示空闲 / 录音 / 收尾 / 错误
- TOML 配置文件 + 热重载
- TUI 配置 + 状态监视界面（`ratatui`） —— **唯一前台 UI**
- 界面双语（zh-CN / en-US），配置项 `ui.language = "auto" | "zh-CN" | "en-US"`，`auto` 读 `$LANG`
- Doctor：启动环境检查 + 终端识别 + 权限引导
- `shuo install` 生成并安装 launchd plist 实现开机自启（binary 安装走 `cargo install` / brew，CLI 不管，详见 [docs/CLI.md](docs/CLI.md)）
- 日志 + 录音历史 + 统计 **全部落到文件**，有稳定 schema，供 TUI 读取，也为未来 GUI 留出唯一数据源

### 3.2 非目标（明确不做）

- 跨平台
- 非语音的输入法功能
- 上 App Store（因此可以自由使用私有 selector）
- ASR 模型自训练
- **段编辑/反悔**（v1 不做；用户说错只能 toggle OFF 重来。属于"编辑能力"，本项目只做"显示 + 上屏"）
- **支持非流式 ASR provider**（OpenAI Whisper API 这种纯 batch 的不入选；本地推理可以包装成流式不算）
- **GUI 配置界面**（v1 不做；但所有内部数据流必须文件化，方便未来挂任意 GUI 上去）
- `.app` Bundle（v1 不做；权限走"授权终端 App"模式）

## 4. 性能与稳定性目标

- 空闲 CPU：< 0.5% 单核（hotkey provider + tokio runtime；cpal/VAD 在 Voice::Idle 时不持有）
- 录音中 CPU：< 5% 单核
- 内存：常驻 < 45MB（单进程，含 AppKit overlay）
- 启动到可录音：< 500ms（不含权限弹窗）
- 一次完整录音 → 上屏：从 toggle OFF 到剪贴板更新 < 1.5s（含 stop_delay + ASR final + pipeline）
- ASR 首字延迟：A 方案 300–500ms（重连）；B 方案 200ms（保活），见 [docs/DESIGN.md §2.9](docs/DESIGN.md#29-客户端-vad--多段-session思考不计费机制)

## 5. 决策记录

### 已定（最新 2026-06-13）

| # | 决策 |
|---|---|
| Q1 | **不做 GUI**。v1 唯一前台 UI 是 TUI。所有数据走文件 + UDS，为未来 GUI 留接口 |
| Q2 | **不打 .app Bundle**。沿用 Go 模式（终端 App 授权）。`shuo install` 生成 launchd plist 开机自启 |
| Q6 | **现在就抽 ASR Provider trait**。首发 DoubaoProvider，未来加新 provider 不动 trait |
| Q8 | **单二进制 + 扁平子命令**。CLI 设计见 [docs/CLI.md](docs/CLI.md) |
| 进程模型 | **单进程 daemon + TUI 客户端**，daemon 常驻、TUI 按需打开 |
| **不拆 overlay 子进程** | AppKit 主线程 + tokio 后台线程同进程跑。NSGlassEffectView 直接在 daemon 进程里渲染（参考 electron-liquid-glass 思路） |
| IPC | **UDS** (`/tmp/shuohua-${UID}.sock`) 实时事件 + 控制命令；**history.jsonl** + **log.jsonl** 持久化。不引入 state.json |
| history 记录范围 | 每次会话都记录（含 `canceled` / `error` / `timeout`），含多段 ASR sessions + pipeline 步骤，不记录原始音频 |
| **history schema** | v1 schema 见 [docs/SCHEMA.md](docs/SCHEMA.md)，含 `version` 字段，扩字段不破坏，删字段才升 version |
| 统计来源 | 全部从 history.jsonl 派生，无独立 stats.json |
| Liquid Glass 变体 | **首选 19 (`control`)**，备选 11 (`bubbles`)。配置项 `overlay.glass_variant` 允许覆写 |
| **删 hold 模式** | 只保留 toggle。dictation 按几十秒不现实；忘按用 `auto_stop_silence_ms` 兜底 |
| ASR 流式 partial | **硬契约必需**。不支持流式 partial 的 provider 不入选；本地批量推理通过 wrapper 满足契约可入选 |
| **ASR 单事件流** | `AsrEvent` enum 单 channel，voice 模块只 select 一臂。删除 `server_side_vad` cap，统一行为 |
| **ASR Hotwords** | `Vec<String>` inline 写在 `config.toml` 的 `[asr].hotwords`；不分级、无 boost 字段。理由：路线图上无 provider 支持 per-word boost（Doubao 接词列表、Whisper 拼 `initial_prompt`、Apple Speech 用 `contextualStrings`）。provider 自由解释，不支持的 caps 标 `hotwords=false`、doctor 提示 |
| ASR 多段 session 模型 | 同一次 Recording 内 client VAD 自动开关 ASR；段间文本累积；段间分隔符可配 |
| **ASR 省钱机制** | A 方案（关 session）v1 选；B 方案（保 session 暂停喂）M3+ 评估 |
| **VAD 实现** | WebRTC VAD（`webrtc-vad` crate）+ 500ms ring buffer pre-roll；RMS 降级；Silero M9 备选 |
| **Doubao 计费维度** | 按音频时长计费（非连接数）。Resource ID `volc.bigasr.sauc.duration`。详见 [docs/DESIGN.md §2.9](docs/DESIGN.md) |
| ASR 段编辑/反悔 | v1 不做（属于"编辑能力"，本项目只做显示+上屏） |
| 中英混合识别 | M2 默认 `Doubao SAUC` + 中英多语模式 + 热词文件（注英文技术词） |
| LLM 后处理 | M2.5 加规则去口语词；M7 加 LLM 清洗（带 App 上下文） |
| **PostProcessor 失败/超时** | 跳过该步，链路继续（**不假设后面会补**）；推 toast 通知；写 pipeline trace 进 history |
| PostProcessor 内容审查 | 不做。该层只清洗，不审查 |
| per-app 配置粒度 | 到 bundle_id 为止，不再细分 URL / input 字段 |
| per-app 配置文件组织 | 一个 app 一个文件：`post/app/<bundle_id>.toml`；找不到 fall back 到 `post/default.toml` |
| **TUI 显示策略** | 默认完整流水线（raw → 每步 output → 最终），不切换模式 |
| **Toast UI 风格** | 与主 Liquid Glass 胶囊同款，底部弹小胶囊，1.5s 自消，不另开 NSPanel |
| Overlay 布局 | 两排：状态/统计/app/chain（第 1 排）+ ASR 实时文字（第 2 排）+ 底部 toast |
| **API key 存储** | 直接明文写在 TOML，仓库放配置模板。`config.toml` 首次写入时 `chmod 0600` |
| **删 `shuo config` 子命令** | `lazyvim` 类编辑器直接编辑文件即可。`validate` 折进 `shuo doctor` |
| **i18n** | 双语 zh-CN/en-US，手写 `t!()` 宏，不引第三方 crate |
| **launchd plist Label** | `com.hza2002.shuohua`（reverse-DNS，参考 yabai `com.koekeishiya.yabai`）|
| **ASR provider 私有配置** | 每个 provider 一份独立 TOML：`~/.config/shuohua/asr/<provider>.toml`，文件名 == provider 名。voice 模块永远不见 provider 私有字段（app_key / language / 厂商特有 flag）。`config.toml` 只写 `[asr] provider = "doubao"` 指路 |
| **ASR 音频 codec** | 由 provider 实现写死，**不暴露给用户**。codec 是工程权衡（CPU/带宽/server 兼容性），用户没足够信息做决定。M2 DoubaoProvider 硬编码 raw PCM；未来若 benchmark 出 opus 显著更优则改 impl，不动配置 |
| **ASR 错误处理** | `AsrError` thiserror enum（`Auth / Network / Quota / Protocol / Timeout / Server / Canceled`），M3+ overlay match enum 分发 toast 样式。**M2 不自动重试**，用户操作可见可重复。Stopping 等 final 超时 5s 后跳过 dispatch + 提示。`Canceled` 静默处理 |
| **Dispatch 触发条件** | 剪贴板 + Cmd+V 只在 `Final`/最后 `Segment` 拼完之后执行。没收到末段就不上屏（部分识别上屏视作 bug）|
| **取消令牌** | `tokio_util::sync::CancellationToken`，Recording 根 token + `.child_token()` 派生子树。Stop 时调一次 root.cancel() 全员收。Go 版 `context.WithCancel` 的 Rust 等价物，DESIGN §2.3 不变量 |
| **音频留存** | `voice.record_audio = false` 默认。开启时落 `~/.local/state/shuohua/audio/<recording_id>.wav`（跟 history.jsonl 同 state dir），文件名 = recording ULID。一次 recording = 一个 wav，多 session 边界从 history.jsonl 时间戳切。不用 `/tmp`/`~/.cache`（语义错位）|
| **Post 路径扁平化** | `~/.config/shuohua/post/<bundle_id>.toml`，去掉原 `app/` 子目录。bundle_id 含 `.` 跟 `default.toml` 视觉不冲突 |

### 仍开放（战术，可在对应里程碑时决定）

| # | 问题 | 候选 | 何时定 |
|---|---|---|---|
| Q3 | 音频 crate | `cpal` / `coreaudio-rs` | M1 写 recorder 时 |
| Q9 | 历史记录是否落音频 wav | 是（按需重放）/ 否（只存文本）| M2 |
| Q10 | ASR 省钱：何时切 B 方案 | M3 实测首字延迟体感后决定 | M3 |
| Q11 | `--help` / `-h` 自动 alias 是否保留 | clap 默认开（接受） | 已选保留 |

## 6. 里程碑（不估工时，按设计完毕后逐个推进）

| M | 目标 | 主要验收 |
|---|---|---|
| **M0** | Liquid Glass demo | 完成；选定变体 19/11 |
| **M1** | 骨架穿透 | CGEventTap 收到 F9，cpal 录 3 秒 PCM 落 wav |
| **M2** | ASR trait + Doubao + 剪贴板 + 上屏（无 VAD） | F9 → DoubaoProvider → 剪贴板 → Cmd+V 完整链路；中英混合 + 热词加载 |
| **M2.5** | 客户端 VAD + 多段 session + RuleBased 去口语词 | 思考几分钟不计费；段间空格拼接；嗯/啊正则去除 |
| **M3** | StateStore + history.jsonl + Overlay 同进程渲染（**两排布局 + 动画 + Toast**） | 录音 → history 一行 jsonl，含 ASR sessions + pipeline；overlay 状态点+文字+时长+字数+app+chain 全部正确切换；Liquid Glass toast 可弹 |
| **M4** | UDS server + `shuo` 智能 fallback 进 TUI 客户端 | 裸跑 `shuo` 连上 daemon 看实时 partial/pipeline_step、滚动历史；关掉 TUI 不影响 daemon |
| **M5** | Doctor + 配置热重载 + launchd 自启 | `shuo install` 写 plist + start，重启后自动起；`shuo doctor` 报权限/配置/ASR 连通 |
| **M6** | Suppress 真实生效 + proptest 覆盖 tracker | 跟 Go 版 perf 对比 |
| **M7** | LLM 后处理（Claude Haiku / GPT-4o-mini）+ App context + per-app 配置文件 | 按当前 App 自动选链路（找不到 fall back default）；失败/超时跳过 + toast 提示；history 记 chain trace |
| **M8** | WhisperCppProvider（whisper-rs，本地离线） | 验证 trait 接口正确；不动 trait |
| **M9** | AppleSpeechProvider（macOS 26 SpeechAnalyzer） | 评估中文质量；决定是否作为默认替代 |

## 7. 文档维护

- 本文档随重大决策更新
- 决策一旦执行（PR 合并），把"开放问题"挪到上方对应章节
- Apple/macOS API 破坏性变化时在 [docs/DESIGN.md §5](docs/DESIGN.md) 不变量列表增加条目
- history schema 字段变化必须升 `version`（见 [docs/SCHEMA.md](docs/SCHEMA.md)）
- 拆分后单文档目标 < 400 行；超过就再拆
