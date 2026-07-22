# shuohua

macOS 15+ Rust 语音输入工具（Apple 本地 ASR 需 26+，低版本用云端 provider）。binary `shuo`，crate `shuohua`。

## 怎么用这套文档（路由，别全读）

按需读，改哪读哪。文档是索引、代码是唯一真相：文档只写「读代码读不出来的东西」——不变量、契约、跨文件边界、扩展配方。

**事实源范围**：除非用户明确指定，不要读取 `docs/archive/` 和 `docs/superpowers/`。
这两个目录都是不进 git 的历史/过程文档：`docs/archive/` 是过时存档，
`docs/superpowers/` 是开发过程中的设计、计划、执行记录。它们可能落后于代码，
不能作为当前事实源；若与当前代码或同步文档冲突，以当前代码和同步文档为准。
其他 `docs/` 下的索引文档才按下表与代码同步维护。查文档优先按下表打开具体文件；
确需搜索时排除 `docs/archive/` 和 `docs/superpowers/`。更新文档只更新同步文档，
不维护这两个历史/过程目录。

| 你要改 | 读 |
|---|---|
| voice 录音/状态机/VAD/取消 | [docs/modules/voice.md](docs/modules/voice.md) |
| 麦克风预处理 backend（webrtc/apple） | [docs/modules/webrtc_backend.md](docs/modules/webrtc_backend.md) / [apple_backend.md](docs/modules/apple_backend.md) |
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
| 发版 | [docs/release.md](docs/release.md) |

库 / API / SDK 文档走 Context7 MCP；Apple/macOS 新 API 查当前文档或本机 SDK interface，不凭记忆写。

## 跨模块约束（修改前读对应文档）

- CGEventTap 在专用 OS 线程运行，经 pipe 把事件桥接到 Tokio；callback 不执行异步业务 → hotkey
- 被吞的 key down/up 必须配对，否则会向前台 App 泄漏键盘状态 → hotkey
- 录音停止必须 drain residual PCM；录音 terminal error / ASR finalize timeout 不执行 post/clipboard/paste → voice
- AppKit 主线程只通过 mpsc 与 Tokio 通信，不在 AppKit callback 中 block future → overlay/architecture
- stale UDS socket 只允许 daemon 持 lock 的启动路径清理；其他入口保守失败 → architecture
- CLI Tokio runtime 只由 `cli::run_command` 创建；子命令不创建或嵌套 runtime → cli
- ASR trait 是 provider 边界；provider 私有配置由 provider 自己加载，不泄漏到 voice 或共享配置 schema → asr

## 验证

- 开发中先跑受影响的最小测试；提交前跑 `make fmt-check` 和相关测试；push / 创建
  PR 前跑 `make check`。完整门禁先更新到最新 Rust stable，再执行 locked fmt /
  clippy / test；合并到 `main` 前必须确认 `CI / check` 通过。
- macOS 权限/录音/上屏体验由用户手动验证；未验证前不宣称阶段完成。
- 处理 bug 先复现/定位，再改最小范围，避免顺手重构。
- 改变不变量、模块边界或对外契约（UDS/history/CLI）时同步对应文档（写什么/不写什么见「开发风格」）。

## 开发配置（只改生效配置，不维护 examples/）

- `~/.config/shuohua/`：`config.toml`、`profile/<id>.toml`、`asr/<id>.toml`、`post/<id>.toml`（asr/post 实例文件内 `type` 指定实现）
- 不替用户启动 GUI 或长驻 daemon；需要真实体验让用户自己跑。

## 开发风格

- 文档只写代码读不出的高层设计（不变量、契约、跨文件边界、扩展配方）；有权威来源的低层信息（字段/默认值/枚举，schema+drift test 已兜底）交代码自证，不复述。
- 文档描述现状，不叙述历史：不写「不再有 X」「旧文件会被拒」等删除/迁移故事，历史方案/决策留 CHANGELOG/git。dev 阶段文档放 `docs/superpowers/`、临时文档用完归 `docs/archive/`（均 gitignored）。
- 谨慎优先：设计/依赖/macOS API 不确定时先验证或停下对齐。
- 源码树只放真实编译运行的东西；scratch/POC 不进 git。
- 注释只写隐藏约束、兼容原因、不明显的不变量；命名能说明的不注释。

## Git workflow

- 开始写入前先看 `git status --short --branch -uall` 和当前分支。若在默认分支
  `main`，先从当前 `main` 创建语义化任务分支（如 `fix/<topic>`、`feat/<topic>`）；
  不直接在默认分支提交，不改写默认分支历史。
- agent 只在任务分支提交。任务分支 commit 使用 `commit.gpgsign=false`；不修改用户
  的全局/仓库 GPG 配置，也不在默认分支绕过签名。最终由用户审核并 merge 到 `main`，
  merge/主分支签名由用户完成。
- 一个阶段一个 commit；commit 标题保持精简，概括本阶段的核心改动。`Cargo.lock`
  进库。详细的改动、取舍和影响写在 commit body，用简要 `-` 分点，一行一点，
  不写流水账。
- 提交前再次看 `git status --short --branch -uall`，只提交本阶段改动。不 push，
  除非用户明确要求。

维护文档与协作默认使用中文；代码标识和技术名词保留英文。
