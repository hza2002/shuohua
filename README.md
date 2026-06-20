# shuohua

macOS 语音输入工具。按下全局热键说话，自动转写并上屏（Cmd+V 粘贴）。Binary 名 `shuo`。

## 平台

- macOS 15+（当前发布 artifact 为 Apple Silicon）
- Apple 本地 ASR provider 需要 macOS 26+；低版本 macOS 请使用云端 ASR provider
- 未签名分发：首次安装与每次升级需要重新授权 Microphone 与 Accessibility 两项权限

## 安装

从 [Releases](https://github.com/HuZiang/shuohua/releases) 下载最新版 `shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz`：

```bash
tar -xzf shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz
cd shuo-vX.Y.Z-aarch64-apple-darwin
xattr -d com.apple.quarantine ./shuo   # 绕 Gatekeeper（首次运行）
mv shuo /usr/local/bin/                # 或放到 PATH 任意位置
shuo doctor                            # 检查权限是否就绪
```

校验 sha256（可选）：

```bash
shasum -a 256 shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz
# 与同名 .sha256 文件内容比对
```

## 权限说明

shuohua 需要授权两项 macOS 系统权限：

- **Microphone**：录音
- **Accessibility**：监听全局热键 + 合成 Cmd+V 上屏

未签名版本每次升级 binary 后这两项需要重新授权（macOS TCC 按 binary 内容 hash 识别未签名程序）。升级后跑 `shuo doctor` 会检测并提示。

## 文档

- [docs/DESIGN.md](docs/DESIGN.md) — 技术设计与架构不变量
- [docs/CLI.md](docs/CLI.md) — CLI 与 launchd 配置
- [docs/SCHEMA.md](docs/SCHEMA.md) — UDS 协议与 history.jsonl
- [docs/MODULES.md](docs/MODULES.md) — 模块边界
- [CHANGELOG.md](CHANGELOG.md) — 变更历史
- [docs/RELEASE.md](docs/RELEASE.md) — 发版手册

## License

[MIT](LICENSE)
