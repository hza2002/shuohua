<!-- Future logo: place a centered project mark above the title. -->
<div align="center">

# shuohua

**A lightweight voice input tool for macOS**

Press a global hotkey, speak, transcribe in real time, optionally refine the text, and insert it into the active app.

[简体中文](README.md) · [Install](#installation) · [Quick start](#quick-start) · [Docs](#documentation)

[![CI](https://github.com/HuZiang/shuohua/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/HuZiang/shuohua/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/HuZiang/shuohua?display_name=tag&sort=semver)](https://github.com/HuZiang/shuohua/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/HuZiang/shuohua/total)](https://github.com/HuZiang/shuohua/releases)
[![License](https://img.shields.io/github/license/HuZiang/shuohua)](LICENSE)
![macOS 15+](https://img.shields.io/badge/macOS-15%2B-000000?logo=apple)
![Apple Silicon](https://img.shields.io/badge/Apple_Silicon-arm64-333333?logo=apple)
![Rust](https://img.shields.io/badge/built_with-Rust-dea584?logo=rust)

</div>

> [!IMPORTANT]
> Current releases are not signed with an Apple Developer ID. Microphone and Accessibility permissions must be granted again after the first install and after every upgrade. See [Permissions](#permissions).

## What it does

```text
Global hotkey  →  Recording and streaming ASR  →  Rules / LLM post-processing  →  Clipboard + paste
     F16               Apple / Doubao                 Per-app profiles               Active cursor
```

- Start, stop, or cancel recording with global hotkeys. The default trigger is `F16`.
- Show live recording, transcription, and post-processing status.
- Use local Apple SpeechAnalyzer on macOS 26+, or Doubao streaming ASR on macOS 15+.
- Build post-processing chains from rules and OpenAI-compatible or Anthropic LLMs.
- Select profiles, hotwords, ASR providers, and processing chains by active app.
- Inspect status, history, configuration, and diagnostics in a terminal UI.
- Hot-reload configuration and choose from built-in light and dark themes.
- Keep audio recording disabled by default. Logs do not contain transcripts, prompts, or clipboard contents.

## Requirements

| Item | Requirement |
|---|---|
| Operating system | macOS 15 or later |
| CPU | Apple Silicon; current releases provide arm64 artifacts only |
| Local ASR | Apple SpeechAnalyzer requires macOS 26+ |
| macOS 15 through 25 | Use a cloud ASR provider such as Doubao |
| Permissions | Microphone and Accessibility |

## Installation

Download the latest `shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz` and matching
`.sha256` file from [GitHub Releases](https://github.com/HuZiang/shuohua/releases/latest).

```bash
# 1. Verify the download
shasum -a 256 shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz
# The output must match the .sha256 file

# 2. Extract and install
tar -xzf shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz
cd shuo-vX.Y.Z-aarch64-apple-darwin
xattr -d com.apple.quarantine ./shuo
sudo install -m 755 ./shuo /usr/local/bin/shuo

# 3. Confirm the command is available
shuo version
```

<details>
<summary>Build from source</summary>

This requires stable Rust, the Xcode 26 SDK, and an Apple Silicon Mac:

```bash
git clone https://github.com/HuZiang/shuohua.git
cd shuohua
cargo build --release
sudo install -m 755 target/release/shuo /usr/local/bin/shuo
```

</details>

## Quick start

### 1. Generate configuration

Configuration lives in `~/.config/shuohua/`. Export the complete commented templates on first use:

```bash
shuo config-template --out ~/.config/shuohua --lang en-US
```

Then select an ASR provider:

- **macOS 26+**: change `provider = "doubao"` to `provider = "apple"` in
  `~/.config/shuohua/profile/default.toml`.
- **macOS 15 through 25**: keep `provider = "doubao"` and add your credentials
  to `~/.config/shuohua/asr/doubao.toml`.

The template command never overwrites existing files. Use an empty directory if you need to export them again.

### 2. Diagnose and grant permissions

```bash
shuo doctor
```

Grant Microphone and Accessibility access as instructed. To exercise the configured ASR and LLM runtime paths:

```bash
shuo doctor --runtime
```

### 3. Install the background service

```bash
shuo install
shuo status
```

`shuo install` installs and starts a per-user launchd service. Then:

1. Focus any text field and press `F16` to start recording.
2. Press `F16` again to stop and finish transcription.
3. The text is copied to the clipboard and pasted with `Cmd+V` by default.
4. Press `Escape` while recording to cancel the current input.
5. Run `shuo` in a terminal to open the status, history, and configuration TUI.

## Permissions

shuohua requires two macOS permissions:

| Permission | Purpose |
|---|---|
| Microphone | Capture speech |
| Accessibility | Observe the global hotkey and synthesize `Cmd+V` |

Current releases are unsigned. macOS TCC identifies unsigned tools by binary content, so replacing the binary during an upgrade normally requires granting both permissions again. Run `shuo doctor` after upgrading and follow its instructions.

> [!NOTE]
> Separate Input Monitoring permission is not required. The current implementation uses Accessibility for the global hotkey capability.

## Commands

| Command | Purpose |
|---|---|
| `shuo` | Open the TUI when the daemon is running, or start the daemon and open the TUI |
| `shuo doctor` | Check permissions, microphone input, configuration, and launchd state |
| `shuo doctor --runtime` | Also exercise configured ASR and LLM providers |
| `shuo config-template` | Export built-in configuration templates and themes |
| `shuo install` | Install and start the launchd service |
| `shuo start` / `stop` / `restart` | Manage the daemon |
| `shuo status` | Show daemon PID, uptime, and recording state |
| `shuo uninstall` | Stop the service and remove its launchd configuration without deleting the binary or user data |

See the [CLI documentation](docs/cli.md) for the complete behavior. The developer documentation is currently written in Chinese.

## Configuration and data

| Path | Contents |
|---|---|
| `~/.config/shuohua/` | Main configuration, profiles, ASR, post-processing, and themes |
| `~/.local/state/shuohua/history/` | Monthly history files |
| `~/.local/state/shuohua/logs/` | Daily diagnostic logs |

- Changes to `config.toml` and themes can be hot-reloaded.
- Profile, ASR, and post-processing files are read at the start of the next recording.
- History is stored as plaintext JSONL and should be treated as sensitive local data.
- Audio is not retained by default; optional FLAC and AAC retention is available.
- Cloud ASR and LLM providers receive the audio or text required by the services you configure.

## Troubleshooting

Start with:

```bash
shuo doctor
shuo status
```

Common fixes:

- Hotkeys or recording stop working after an upgrade: grant Microphone and Accessibility again.
- Configuration fails to load: use the exact file and field reported by `shuo doctor`.
- The daemon is unhealthy: run `shuo restart`, then inspect
  `~/.local/state/shuohua/logs/`.
- Apple ASR is unavailable on macOS 25 or earlier: switch to a cloud provider.

See the [troubleshooting guide](docs/debug.md) for more steps. If the problem is reproducible, open an
[issue](https://github.com/HuZiang/shuohua/issues) with your macOS version, `shuo version`, and redacted diagnostics.

## Documentation

`docs/` is intended for maintainers and users who want implementation details. It is currently available in Chinese:

- [Architecture and data flow](docs/architecture.md)
- [CLI and launchd](docs/cli.md)
- [Configuration and hot reload](docs/modules/config.md)
- [ASR providers](docs/modules/asr.md)
- [Voice state machine and VAD](docs/modules/voice.md)
- [Hotkeys](docs/modules/hotkey.md)
- [Post-processing](docs/modules/post.md)
- [Overlay](docs/modules/overlay.md)
- [Protocols and data formats](docs/schema.md)
- [Troubleshooting](docs/debug.md)
- [Changelog](CHANGELOG.md)

## Development

```bash
cargo fmt
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```

Real macOS permission, recording, and automatic-paste behavior must be verified manually on a Mac.

## License

[MIT](LICENSE)
