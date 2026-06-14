# shuohua

macOS 26+ 的 Rust 语音输入工具。binary 名 `shuo`，crate 名 `shuohua`。

## 必读文档

每个新 session 开始时按顺序读：

1. [REQUIREMENTS.md](REQUIREMENTS.md) — 需求 + 决策表 + 里程碑
2. [docs/DESIGN.md](docs/DESIGN.md) — 技术设计 + 不变量
3. [docs/SCHEMA.md](docs/SCHEMA.md) — UDS 协议 + history.jsonl
4. [docs/CLI.md](docs/CLI.md) — CLI + launchd
5. [docs/MODULES.md](docs/MODULES.md) — 已实现 vs 待引入

库 / API / SDK 文档查询走 Context7 MCP（全局 `~/.claude/CLAUDE.md` 规则）。

## 开发阶段配置

- 开发阶段不要维护 `examples/config/` 示例配置。
- 所有配置调试都直接改真正生效的用户配置：
  - `~/.config/shuohua/config.toml`
  - `~/.config/shuohua/asr/<provider>.toml`
- 原因：当前 overlay / ASR / post 参数仍在快速试验，示例配置容易和真实配置漂移，造成误判。
- 只有准备 release 时，才从当时确认可用的真实配置生成 `examples/config/` 下的示例配置，并补充面向用户的注释。

## 开发风格

- **谨慎优先**：设计选择不明确、依赖能力存疑、macOS API 用法不确定 → 停下问用户，不要拍脑袋。
- **每个文件诞生时就明确职责**：不塞模板代码、不预先 mkdir 占位空模块。未来路径登记在 [docs/MODULES.md](docs/MODULES.md)，源码树只放真编译的东西。
- **不写无意义注释**：WHAT 留给好命名；WHY（隐藏约束、subtle invariant、绕过特定 bug）才写。
- **依赖延迟引入**：不为了"看起来现代"提前加，到真需要的里程碑再引。
- **测试边界**：纯函数单测 + I/O 边界 fake。每个里程碑的核心纯函数模块至少有单测。
- **验证前别说"完成"**：`cargo check` + `cargo test` 通过 + 用户手动验过应用，才算里程碑完成。

## Git workflow

- 一个里程碑一个 commit。Commit message 第一行 ≤72 字符摘要成果；正文解释做了什么 + 为什么 + 哪部分留给下一里程碑。
- 仓库 `commit.gpgsign=false`。
- `Cargo.lock` 进库（binary crate 约定）。
- 大改动前讨论；动之前先 `git status`。

## 协作偏好

- 中文为主，技术名词混英文 OK。
- 拍板式：列选项 + 推荐 + 理由，让用户选；不替用户做不可逆设计决定。
- 不替用户启动 GUI / 长驻进程；用户自己跑应用。
- 环境：macOS 26、Apple Silicon、F16 物理键来自外接键盘。
