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
     热键      →    识别    →   处理    →   输出
右 Option 双击 → Apple/云端 → 规则/LLM  → 自动粘贴
```

- 双击右 Option 开始或停止录音，按 Escape 取消，Shift+双击右 Option 续接上次转写；开始/取消/续写三个全局快捷键均可修改。
- 实时显示录音、识别和后处理状态。
- macOS 15+ 可使用豆包、腾讯云等云端 ASR；macOS 26+ 还可选择 Apple 本地 SpeechAnalyzer。
- 默认使用原始麦克风采集；可切换到 macOS 系统语音处理采集作为噪声、回声或增益问题的兜底。
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
| 云端 ASR | macOS 15+ 可使用豆包、腾讯云等云端 provider |
| Apple 本地 ASR | SpeechAnalyzer 仅在 macOS 26+ 可用 |
| 权限 | Microphone、Accessibility |

## 安装

从 [GitHub Releases](https://github.com/hza2002/shuohua/releases/latest) 下载最新的
`shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz` 和同名 `.sha256` 文件。

```bash
# 1. 校验下载文件
shasum -a 256 -c shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz.sha256

# 2. 解压并安装
tar -xzf shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz
cd shuo-vX.Y.Z-aarch64-apple-darwin
xattr -d com.apple.quarantine ./shuo
mkdir -p ~/.local/bin
install -m 755 ./shuo ~/.local/bin/shuo

# 3. 确认命令可用
shuo version
```

`~/.local/bin/shuo` 是唯一受支持的安装路径（per-user，无需 sudo）；请确认
`~/.local/bin` 已在 `PATH` 中。`shuo update` 始终把新 binary 装到这个路径（必要时
自动创建 `~/.local/bin`，永不 sudo 写系统目录）；若你是从别处运行的 binary，更新
仍会写入 preferred 路径并提示你迁移（调整 PATH、`shuo service install` 重新指向、
自行删除旧 binary）。装错位置时 `shuo doctor` / `shuo service status` 会报路径漂移。

<details>
<summary>从源码构建</summary>

需要 Rust stable、Xcode 26 SDK 和 Apple Silicon Mac：

```bash
git clone https://github.com/hza2002/shuohua.git
cd shuohua
cargo build --release
mkdir -p ~/.local/bin
install -m 755 target/release/shuo ~/.local/bin/shuo
```

</details>

## 快速开始

### 1. 生成配置

配置文件位于 `~/.config/shuohua/`。首次使用可导出带注释的完整模板：

```bash
shuo config-template --out ~/.config/shuohua --lang zh-CN
```

默认快捷键是双击右 Option（PC 键盘通常标为右 Alt），可编辑
`~/.config/shuohua/config.toml` 修改：

```toml
[hotkey]
# 修饰键组合
trigger = "ctrl+shift+space"
# 双击按键
# trigger = "right_option:double"
# cancel = "escape"                      # 取消本次录音
# resume = "shift+right_option:double"   # 从上次转写续写
```

然后选择 ASR。profile 通过 `[asr].instance` 引用一个 ASR 实例，实例文件
`~/.config/shuohua/asr/<name>.toml` 用 `type` 指定 provider（模板已生成
`doubao` / `tencent` / `apple` 三个实例）：

- **所有支持的 macOS 版本**：默认 profile（`~/.config/shuohua/profile/default.toml`）
  的 `[asr].instance = "doubao"`，在实例文件 `~/.config/shuohua/asr/doubao.toml`
  填入豆包凭据。当前 provider 使用 `app_key` / `access_key` 鉴权，获取方式见豆包语音
  [旧版控制台快速入门](https://www.volcengine.com/docs/6561/163043)，协议参数见
  [大模型流式语音识别 API](https://www.volcengine.com/docs/6561/1354869)。
- **腾讯云**：把 `instance` 改为 `"tencent"`，在 `~/.config/shuohua/asr/tencent.toml`
  填入 `app_id` / `secret_id` / `secret_key`。
- **macOS 26+**：把 `instance` 改为 `"apple"`，使用本地 SpeechAnalyzer（实例
  `~/.config/shuohua/asr/apple.toml` 已由模板生成，无需凭据）。

可选麦克风预处理在 `~/.config/shuohua/config.toml` 的
`voice.preprocess.backend` 配置。默认 `webrtc`（WebRTC 降噪/高通/数字增益，启动开销接近
`off`，已内置到二进制）；`off` 用原始采集、不做任何处理（需要保留原始音频时用它）；
`apple` 用 macOS 原生语音处理（回声消除/降噪/增益），效果好但每次启动建立连接会慢一点。

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
shuo service install
shuo service status
```

`shuo service install` 会安装并启动当前用户的 launchd 服务。之后：

1. 在任意输入框双击右 Option（右 Alt）开始录音。
2. 再双击一次右 Option（右 Alt）停止并等待转写。
3. 文本会写入剪贴板，并默认通过 `Cmd+V` 自动粘贴。
4. 录音过程中按 `Escape` 可取消本次输入。
5. 取消或识别超时留下文本后，按 `Shift`+双击右 Option 可从上次结果续写。
6. 在终端运行 `shuo` 可打开状态、历史和配置 TUI。

## 权限

shuohua 只需要两项 macOS 系统权限：

| 权限 | 用途 |
|---|---|
| Microphone | 采集语音 |
| Accessibility | 监听全局热键并模拟 `Cmd+V` |

当前 Release 未签名。macOS TCC 会按 binary 内容识别这类程序，因此升级 binary
后通常需要重新授权以上两项权限。升级后运行 `shuo doctor`，按提示处理即可。

后续升级可运行：

```bash
shuo update
shuo service restart
shuo doctor
```

`shuo update` 下载的 binary 不带 `com.apple.quarantine`，无需再 `xattr`。若你改用
浏览器手动重新下载 tarball 覆盖安装，新 binary 会带 quarantine，需再执行一次
`xattr -d com.apple.quarantine ~/.local/bin/shuo`。

> [!NOTE]
> 不需要单独授予 Input Monitoring。当前实现使用 Accessibility 覆盖全局热键所需能力。

## 常用命令

| 命令 | 作用 |
|---|---|
| `shuo` | daemon 已运行时打开 TUI；未运行时启动 daemon 并打开 TUI |
| `shuo doctor` | 检查权限、麦克风、配置和 launchd 状态 |
| `shuo doctor --runtime` | 额外检查已配置的 ASR 和 LLM provider |
| `shuo config-template` | 导出内置配置模板和主题 |
| `shuo completions <shell>` | 生成 zsh、bash 或 fish completion 脚本 |
| `shuo update` | 检查并更新当前 shuo binary |
| `shuo service install` | 安装并启动后台服务 |
| `shuo service start` / `stop` / `restart` | 管理后台服务 |
| `shuo service status` | 查看 daemon PID、运行时长和录音状态 |
| `shuo service uninstall` | 停止服务并移除 launchd 配置，不删除 binary 和用户数据 |

completion 脚本输出到 stdout。Homebrew 环境的 zsh 手动安装示例：

```bash
shuo completions zsh > "$(brew --prefix)/share/zsh/site-functions/_shuo"
```

完整说明见 [CLI 文档](docs/cli.md)。

## 配置与数据

| 路径 | 内容 |
|---|---|
| `~/.config/shuohua/` | 主配置、profile、ASR、post、theme |
| `~/.local/state/shuohua/history/` | 按月分片的历史记录 |
| `~/.local/state/shuohua/audio/` | 可选保留的 FLAC 或 AAC 录音 |
| `~/.local/state/shuohua/logs/` | 按日分片的诊断日志 |

- `config.toml` 和主题变更可热重载。
- profile、ASR 和 post 配置在下一次录音开始时读取。
- history 以明文 JSONL 保存，请按本机敏感数据管理。
- 录音默认不落盘；启用 `voice.record_audio` 后写入 `audio/`，可选 FLAC 或 AAC。
- 使用云端 ASR 或 LLM 时，相关音频或文本会发送给你配置的第三方服务。

### 卸载

没有「一键卸载」命令——shuo 只有一个 binary、一份 launchd plist 和纯文本配置/数据，
手动删即可（history 是你唯一的数据源，默认保留，确实要删再删）：

```bash
shuo service uninstall            # 停服务并删 launchd plist（不碰 binary/数据）
rm ~/.local/bin/shuo             # 删 binary
rm -rf ~/.config/shuohua         # 删配置（profile/ASR/post/theme）
rm -rf ~/.local/state/shuohua    # 删 history/audio/logs —— 不可恢复
```

设了 `XDG_CONFIG_HOME` / `XDG_STATE_HOME` 时，配置与数据在对应目录下的 `shuohua/`。

## 排障

先运行：

```bash
shuo doctor
shuo service status
```

常见处理：

- 升级后热键或录音失效：重新授权 Microphone 与 Accessibility。
- 配置无法加载：查看 `shuo doctor` 输出中的具体文件和字段。
- daemon 异常：执行 `shuo service restart`，再查看
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
- [发版](docs/release.md)

## 开发

```bash
cargo fmt                                  # 格式化 Rust 源码
cargo check                                # 快速编译检查
cargo test                                 # 运行全部测试
cargo clippy --all-targets -- -D warnings  # 静态检查，warning 视为错误
```

真实的 macOS 权限、录音和自动粘贴体验需要在本机手动验证。

## License

[MIT](LICENSE)
