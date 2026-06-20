# shuohua

macOS 26+ 的 Rust 语音输入工具。binary 名 `shuo`，crate 名 `shuohua`。

## 必读文档

每个新 session 开始时按顺序读：

1. [docs/DESIGN.md](docs/DESIGN.md) - 技术设计、架构不变量、测试边界
2. [docs/SCHEMA.md](docs/SCHEMA.md) - UDS 协议 + history.jsonl
3. [docs/CLI.md](docs/CLI.md) - CLI + launchd
4. [docs/MODULES.md](docs/MODULES.md) - 已实现模块和当前源码边界
5. [CHANGELOG.md](CHANGELOG.md) - 阶段性历史和重要取舍

库 / API / SDK 文档查询走 Context7 MCP。涉及 Apple / macOS 新 API 时尤其要查当前文档或本机 SDK interface，不能凭记忆写。

## 开发配置

- 开发阶段不维护 `examples/` 配置样例；真实调试只改生效配置：
  - `~/.config/shuohua/config.toml`
  - `~/.config/shuohua/profile/*.toml`
  - `~/.config/shuohua/asr/<provider>.toml`
  - `~/.config/shuohua/post/**/*.toml`
- 示例配置只在准备公开发布时，从当时确认可用的真实配置重新生成。
- 不替用户启动 GUI 或长驻 daemon；需要真实体验时让用户自己跑。

## 开发风格

- 简单优先：小 API、低抽象、行为显式，避免为未来假设提前铺层。
- 谨慎优先：设计选择不明确、依赖能力存疑、macOS API 用法不确定时，先验证或停下对齐。
- 源码树只放真实编译/运行的东西；scratch、tmp、一次性 POC 不进 git。
- 注释只写隐藏约束、兼容性原因和不明显的不变量；命名能说明的不要注释。
- 每个 provider 自己加载 `~/.config/shuohua/asr/<provider>.toml`，voice/config 层不见 provider 私有字段。
- ASR trait 是边界，新增 provider 不改 `AsrProvider` / `AsrSession`，除非先重写设计并讨论。
- history schema 字段变化先更新 [docs/SCHEMA.md](docs/SCHEMA.md)，删字段或破坏兼容才升 version。

## 测试与验证

- 核心纯函数模块必须有单测；I/O 边界用 fake 或小范围集成测试。
- 改代码后至少跑 `cargo fmt`、`cargo check`、`cargo test`。
- macOS 真实权限、录音、上屏体验必须由用户手动验证；验证前不要宣称阶段完成。
- 处理 bug 时先复现/定位，再改最小范围，避免顺手重构。

## Git workflow

- 一个阶段一个 commit；commit message 第一行不超过 72 字符。
- 仓库 `commit.gpgsign=false`。
- `Cargo.lock` 进库。
- 提交前看 `git status --short --branch -uall`，只提交本阶段相关改动。
- 不 push，除非用户明确要求。

## 协作偏好

- 中文为主，技术名词可用英文。
- 需要用户拍板时给选项、推荐和理由；不可逆设计不擅自替用户决定。
- 环境：macOS 26、Apple Silicon、F16 物理键来自外接键盘。
