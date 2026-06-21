# shuohua

macOS 15+ Rust 语音输入工具（Apple 本地 ASR 需 26+，低版本用云端 provider）。binary `shuo`，crate `shuohua`。

## 怎么用这套文档（路由，别全读）

按需读，改哪读哪。文档只写「读代码读不出来的东西」——不变量、契约、跨文件边界、扩展配方。

| 你要改 | 读 |
|---|---|
| voice 录音/状态机/VAD/取消 | [docs/modules/voice.md](docs/modules/voice.md) |
| hotkey 语法/suppress | [docs/modules/hotkey.md](docs/modules/hotkey.md) |
| ASR provider（含新增） | [docs/modules/asr.md](docs/modules/asr.md) |
| post 处理链/profile 路由 | [docs/modules/post.md](docs/modules/post.md) |
| overlay 视觉/动画/平台边界 | [docs/modules/overlay.md](docs/modules/overlay.md) |
| 配置/热重载/theme | [docs/modules/config.md](docs/modules/config.md) |
| TUI（Status/History/Configure）/ IPC server | [docs/architecture.md](docs/architecture.md)（数据流）+ 代码 `src/tui`、`src/ipc` |
| i18n 文案/语言/伪语言 | [docs/architecture.md](docs/architecture.md)（§7 i18n） |
| UDS / history / audio / trace 格式 | [docs/schema.md](docs/schema.md) |
| CLI / launchd | [docs/cli.md](docs/cli.md) |
| 进程模型/线程/选型/数据流/i18n/安全 | [docs/architecture.md](docs/architecture.md) |
| 排障 | [docs/debug.md](docs/debug.md) |
| 发版 | [docs/ops/release.md](docs/ops/release.md) |

库 / API / SDK 文档走 Context7 MCP；Apple/macOS 新 API 查当前文档或本机 SDK interface，不凭记忆写。

## 不可违反的不变量（破坏前先读对应模块文档）

1. CGEventTap 回调跑专用 OS 线程 CFRunLoop，不让出 → hotkey
2. C→Rust 事件桥用 pipe，不用 cgo callback → hotkey
3. 录音停止必 drain residual + `stop_delay_ms`(800ms)，否则尾字被切 → voice
4. `notify` 监听配置目录，不监听文件本身（编辑器换 inode） → config
5. `NSGlassEffectView` 必须作子视图，不作 contentView → overlay
6. 热键注册在系统启动前完成；运行时新增要保证 dispatcher 已起 → hotkey
7. 热键 down/up 配对吞，漏吞会让前台 App modifier 状态泄漏 → hotkey
8. AppKit 主线程与 tokio 用 mpsc 通信，绝不在 AppKit callback 里 block tokio future → overlay/architecture
9. `frontmostApplication` 在 toggle OFF 瞬间取一次缓存，不在 PostProcessor 内反复取 → post
10. Stale UDS socket 只有持 lock 的 daemon 启动路径能清理，其余保守失败 → architecture
11. `Voice::Idle` 录音资源按模式分：Continuous 不持 stream，VadPause Idle 持 stream 但不发 PCM → voice
12. 麦克风可用性靠运行时 watchdog，不靠预检 → voice
13. Error/Timeout 路径不上屏不写剪贴板 → voice

## 验证（改完必跑）

- `cargo fmt && cargo check && cargo test`
- macOS 权限/录音/上屏体验由用户手动验证；未验证前不宣称阶段完成。
- 处理 bug 先复现/定位，再改最小范围，避免顺手重构。
- **何时更新文档**：改动涉及以下之一时，同步更新对应 `docs/modules/*.md`：①新增/删除/改变了不变量 ②改变了模块间边界 ③改变了对外契约（UDS/history/CLI）。纯实现细节、bugfix（不变量不改）、重构（行为不变）不需要动文档。

## 开发配置（只改生效配置，不维护 examples/）

- `~/.config/shuohua/`：`config.toml`、`profile/*.toml`、`asr/<provider>.toml`、`post/**/*.toml`
- 不替用户启动 GUI 或长驻 daemon；需要真实体验让用户自己跑。

## 开发风格

- 简单优先：小 API、低抽象、行为显式，不为未来假设提前铺层。
- 文档只写代码读不出来的东西；复述代码、已拍板的历史方案不进文档（决策留 CHANGELOG/git）。
- 谨慎优先：设计/依赖/macOS API 不确定时先验证或停下对齐。
- 源码树只放真实编译运行的东西；scratch/POC 不进 git。
- 注释只写隐藏约束、兼容原因、不明显的不变量；命名能说明的不注释。
- provider 私有字段各自加载，voice/config 层不见；ASR trait 是边界，新增 provider 不改 trait（要改先重写设计并讨论）。

## Git workflow

- 一个阶段一个 commit；首行 ≤72 字符。`commit.gpgsign=false`。`Cargo.lock` 进库。
- 提交前看 `git status --short --branch -uall`，只提交本阶段改动。不 push，除非用户明确要求。

## 协作偏好

- 中文为主，技术名词可用英文。
- 需要拍板时给选项、推荐和理由；不可逆设计不擅自替用户决定。
- 环境：macOS 26、Apple Silicon、F16 来自外接键盘。
