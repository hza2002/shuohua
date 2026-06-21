<div align="center">

<img src="assets/icon/shuohua-icon.svg" width="168" alt="shuohua logo">

# shuohua

**面向 macOS 的轻量语音输入工具**

按下全局热键说话，实时转写、按需润色，然后自动输入到当前应用。

[English](README.en.md) · [安装](#安装) · [快速开始](#快速开始) · [文档](#文档)

[![CI](https://github.com/hza2002/shuohua/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/hza2002/shuohua/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/hza2002/shuohua?display_name=tag&sort=semver)](https://github.com/hza2002/shuohua/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/hza2002/shuohua/total)](https://github.com/hza2002/shuohua/releases)
[![License](https://img.shields.io/github/license/hza2002/shuohua)](LICENSE)
![macOS 15+](https://img.shields.io/badge/macOS-15%2B-000000?logo=apple)
![Apple Silicon](https://img.shields.io/badge/Apple_Silicon-arm64-333333?logo=apple)
![Rust](https://img.shields.io/badge/built_with-Rust-dea584?logo=rust)

</div>

> [!IMPORTANT]
> 当前 Release 未做 Apple Developer ID 签名。首次安装和每次升级后，都需要重新授权 Microphone 与 Accessibility。详见[权限](#权限)。

## 它能做什么

```text
全局热键  →  录音与实时 ASR  →  规则 / LLM 后处理  →  剪贴板 + 自动粘贴
    F16          Apple / 豆包          可按应用切 profile          当前光标
```

- 全局热键开始、停止或取消录音，默认使用 `F16`。
- 实时显示录音、识别和后处理状态。
- 支持 macOS 26+ 的 Apple 本地 SpeechAnalyzer，以及适用于 macOS 15+ 的豆包流式 ASR。
- 支持规则和 OpenAI-compatible / Anthropic LLM 后处理链。
- 可按前台应用选择不同 profile、热词、ASR 和后处理配置。
- 提供 TUI 状态页、历史记录、配置浏览和诊断。
- 配置和主题可热重载，内置多套明暗主题。
- 默认不保存录音；日志不记录识别正文、prompt 或剪贴板内容。

## 平台要求

| 项目 | 要求 |
|---|---|
| 操作系统 | macOS 15 或更高版本 |
| CPU | Apple Silicon（当前 Release 仅提供 arm64 artifact） |
| 本地 ASR | Apple SpeechAnalyzer 需要 macOS 26+ |
| macOS 15 至 25 | 使用豆包等云端 ASR provider |
| 权限 | Microphone、Accessibility |

## 安装

从 [GitHub Releases](https://github.com/hza2002/shuohua/releases/latest) 下载最新的
`shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz` 和同名 `.sha256` 文件。

```bash
# 1. 校验下载文件
shasum -a 256 shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz
# 输出应与 .sha256 文件一致

# 2. 解压并安装
tar -xzf shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz
cd shuo-vX.Y.Z-aarch64-apple-darwin
xattr -d com.apple.quarantine ./shuo
sudo install -m 755 ./shuo /usr/local/bin/shuo

# 3. 确认命令可用
shuo version
```

<details>
<summary>从源码构建</summary>

需要 Rust stable、Xcode 26 SDK 和 Apple Silicon Mac：

```bash
git clone https://github.com/hza2002/shuohua.git
cd shuohua
cargo build --release
sudo install -m 755 target/release/shuo /usr/local/bin/shuo
```

</details>

## 快速开始

### 1. 生成配置

配置文件位于 `~/.config/shuohua/`。首次使用可导出带注释的完整模板：

```bash
shuo config-template --out ~/.config/shuohua --lang zh-CN
```

然后选择 ASR：

- **macOS 26+**：把 `~/.config/shuohua/profile/default.toml` 中的
  `provider = "doubao"` 改为 `provider = "apple"`。
- **macOS 15 至 25**：保留 `provider = "doubao"`，并在
  `~/.config/shuohua/asr/doubao.toml` 填入豆包凭据。

模板命令不会覆盖已有文件。需要重新生成时，请指定一个空目录。

### 2. 检查环境并授权

```bash
shuo doctor
```

根据输出授予 Microphone 和 Accessibility 权限。需要实际检查 ASR、LLM provider
运行路径时执行：

```bash
shuo doctor --runtime
```

### 3. 安装后台服务

```bash
shuo install
shuo status
```

`shuo install` 会安装并启动当前用户的 launchd 服务。之后：

1. 在任意输入框按 `F16` 开始录音。
2. 再按一次 `F16` 停止并等待转写。
3. 文本会写入剪贴板，并默认通过 `Cmd+V` 自动粘贴。
4. 录音过程中按 `Escape` 可取消本次输入。
5. 在终端运行 `shuo` 可打开状态、历史和配置 TUI。

## 权限

shuohua 只需要两项 macOS 系统权限：

| 权限 | 用途 |
|---|---|
| Microphone | 采集语音 |
| Accessibility | 监听全局热键并模拟 `Cmd+V` |

当前 Release 未签名。macOS TCC 会按 binary 内容识别这类程序，因此升级 binary
后通常需要重新授权以上两项权限。升级后运行 `shuo doctor`，按提示处理即可。

> [!NOTE]
> 不需要单独授予 Input Monitoring。当前实现使用 Accessibility 覆盖全局热键所需能力。

## 常用命令

| 命令 | 作用 |
|---|---|
| `shuo` | daemon 已运行时打开 TUI；未运行时启动 daemon 并打开 TUI |
| `shuo doctor` | 检查权限、麦克风、配置和 launchd 状态 |
| `shuo doctor --runtime` | 额外检查已配置的 ASR 和 LLM provider |
| `shuo config-template` | 导出内置配置模板和主题 |
| `shuo install` | 安装并启动 launchd 服务 |
| `shuo start` / `stop` / `restart` | 管理 daemon |
| `shuo status` | 查看 daemon PID、运行时长和录音状态 |
| `shuo uninstall` | 停止服务并移除 launchd 配置，不删除 binary 和用户数据 |

完整说明见 [CLI 文档](docs/cli.md)。

## 配置与数据

| 路径 | 内容 |
|---|---|
| `~/.config/shuohua/` | 主配置、profile、ASR、post、theme |
| `~/.local/state/shuohua/history/` | 按月分片的历史记录 |
| `~/.local/state/shuohua/logs/` | 按日分片的诊断日志 |

- `config.toml` 和主题变更可热重载。
- profile、ASR 和 post 配置在下一次录音开始时读取。
- history 以明文 JSONL 保存，请按本机敏感数据管理。
- 录音默认不落盘，可配置为 FLAC 或 AAC。
- 使用云端 ASR 或 LLM 时，相关音频或文本会发送给你配置的第三方服务。

## 排障

先运行：

```bash
shuo doctor
shuo status
```

常见处理：

- 升级后热键或录音失效：重新授权 Microphone 与 Accessibility。
- 配置无法加载：查看 `shuo doctor` 输出中的具体文件和字段。
- daemon 异常：执行 `shuo restart`，再查看
  `~/.local/state/shuohua/logs/`。
- Apple ASR 在 macOS 25 或更低版本不可用：切换到云端 provider。

更多步骤见 [排障文档](docs/debug.md)。如果问题仍能复现，请提交
[Issue](https://github.com/hza2002/shuohua/issues)，并附上 macOS 版本、`shuo version`
和已脱敏的诊断信息。

## 文档

`docs/` 面向维护者和希望深入了解实现的用户，目前仅提供中文：

- [架构与数据流](docs/architecture.md)
- [CLI 与 launchd](docs/cli.md)
- [配置与热重载](docs/modules/config.md)
- [ASR provider](docs/modules/asr.md)
- [语音状态机与 VAD](docs/modules/voice.md)
- [热键](docs/modules/hotkey.md)
- [后处理链](docs/modules/post.md)
- [Overlay](docs/modules/overlay.md)
- [协议与数据格式](docs/schema.md)
- [排障](docs/debug.md)
- [变更历史](CHANGELOG.md)
- [发版](RELEASE.md)

## 开发

```bash
cargo fmt
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```

真实的 macOS 权限、录音和自动粘贴体验需要在本机手动验证。

## License

[MIT](LICENSE)
