## 安装

下载下方 `shuo-*-aarch64-apple-darwin.tar.gz`：

```bash
tar -xzf shuo-*-aarch64-apple-darwin.tar.gz
cd shuo-*-aarch64-apple-darwin
xattr -d com.apple.quarantine ./shuo
mv shuo /usr/local/bin/
shuo doctor
```

校验 sha256：与同名 `.sha256` 文件内容比对。

## ⚠️ 权限说明（必读）

本版本**未做 Apple Developer ID 签名**，首次安装与每次升级都需要重新授权两项权限：

- **Microphone**（录音）
- **Accessibility**（监听全局热键 + 合成 Cmd+V 上屏）

升级后跑 `shuo doctor` 会检测并提示需要重新授权的项。这是 macOS TCC 对未签名程序的默认行为，详细原因见 [RELEASE.md](https://github.com/hza2002/shuohua/blob/main/RELEASE.md)。

## 变更内容

见 [CHANGELOG.md](https://github.com/hza2002/shuohua/blob/main/CHANGELOG.md) 本版本对应段落。
