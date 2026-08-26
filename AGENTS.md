# shuohua

面向 macOS 的 Rust 语音输入工具。binary 是 `shuo`，crate 是 `shuohua`。

## 文档路由

代码是实现事实源，文档只记录代码无法表达的契约和跨文件边界。按任务读取对应文档，不要一次加载全部内容。

| 修改范围 | 读取 |
|---|---|
| 录音、状态机、VAD、取消 | [docs/modules/voice.md](docs/modules/voice.md) |
| 麦克风预处理 backend | [docs/modules/webrtc_backend.md](docs/modules/webrtc_backend.md)、[docs/modules/apple_backend.md](docs/modules/apple_backend.md) |
| hotkey 语法和 suppress | [docs/modules/hotkey.md](docs/modules/hotkey.md) |
| ASR provider | [docs/modules/asr.md](docs/modules/asr.md) |
| post 处理链和 profile 路由 | [docs/modules/post.md](docs/modules/post.md) |
| overlay 视觉、动画和平台边界 | [docs/modules/overlay.md](docs/modules/overlay.md) |
| 配置、热重载和 theme | [docs/modules/config.md](docs/modules/config.md) |
| TUI、IPC、进程模型、i18n 和安全 | [docs/architecture.md](docs/architecture.md) |
| UDS、history、audio 和 trace 格式 | [docs/schema.md](docs/schema.md) |
| CLI 和 launchd | [docs/cli.md](docs/cli.md) |
| 排障 | [docs/debug.md](docs/debug.md) |
| 发版 | [docs/release.md](docs/release.md) |

Apple 和 macOS 新 API 以当前官方文档或本机 SDK interface 为准，不凭记忆实现。

## 工作边界

- 排障时可以读取 `~/.config/shuohua/` 以及 `~/.local/state/shuohua/` 下的 history、audio、traces 和 logs。未经用户明确要求，不修改或删除这些运行数据。
- 不替用户启动 GUI 或常驻 daemon。需要验证 macOS 权限、录音或上屏体验时，由用户运行并反馈结果。
- 文档只描述当前契约。改变不变量、模块边界或 UDS、history、CLI 等对外契约时，同步对应的正式文档；历史留在 Git 和 `CHANGELOG.md`。

## 验证

- 开发中先跑受影响的最小测试。
- 提交前运行 `make fmt-check` 和相关测试。
- push 或创建 PR 前运行 `make check`，并确认 `CI / check` 通过。
- 未完成对应的 macOS 手动验证前，不宣称运行时体验已经验证。

## Git Workflow

- 写入前检查 `git status --short --branch -uall` 和当前分支。位于 `main` 时，先创建语义化任务分支。
- 需要提交时，一阶段一个 commit；任务分支使用 `commit.gpgsign=false`，不修改用户的 Git 签名配置。
- 不改写 `main` 历史，不在 `main` 上提交。最终 merge 由用户完成，除非用户明确要求，否则不 push。

维护文档与协作默认使用中文，代码标识和技术名词保留英文。
