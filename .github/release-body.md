## 安装

下载下方 `shuo-*-aarch64-apple-darwin.tar.gz`：

```bash
tar -xzf shuo-*-aarch64-apple-darwin.tar.gz
cd shuo-*-aarch64-apple-darwin
xattr -d com.apple.quarantine ./shuo
mkdir -p ~/.local/bin
install -m 755 ./shuo ~/.local/bin/shuo
shuo doctor
```

请确认 `~/.local/bin` 已在 `PATH` 中。

校验 SHA256：

```bash
shasum -a 256 -c shuo-*-aarch64-apple-darwin.tar.gz.sha256
```

后续升级可运行：

```bash
shuo update
shuo service restart
shuo doctor
```

## ⚠️ 权限说明（必读）

本版本**未做 Apple Developer ID 签名**，首次安装与每次升级都需要重新授权两项权限：

- **Microphone**（录音）
- **Accessibility**（监听全局热键 + 合成 Cmd+V 上屏）

升级后跑 `shuo doctor` 会检测并提示需要重新授权的项。这是 macOS TCC 对未签名程序的默认行为，详细原因见 [release.md](https://github.com/hza2002/shuohua/blob/main/docs/release.md)。

## 变更内容

见 [CHANGELOG.md](https://github.com/hza2002/shuohua/blob/main/CHANGELOG.md) 本版本对应段落。
