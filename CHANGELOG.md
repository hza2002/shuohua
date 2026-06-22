# Changelog

本文件只记录公开发布版本的用户可感知变化，最新版本在最上面。

## v0.1.2 - 2026-06-22

### Changed

- Exported configuration templates now default to Silero VAD with a longer post-processing timeout.
- Exported Doubao templates now enable VAD pause so the starter configuration can pause and resume ASR sessions.
- Exported profile templates now include default, chat, and agent profiles using Doubao ASR and the DeepSeek post-processing chain.
- Default app routes for chat and agent profiles are more conservative and avoid duplicate bundle IDs that could match multiple profiles.

## v0.1.1 - 2026-06-22

### Changed

- New configuration templates use a double-tap of Right Option as the default recording hotkey, with clearer setup and platform guidance in the documentation.

### Fixed

- Fixed `shuo doctor` crashing when its timeout checks ran without an active Tokio runtime.

## v0.1.0 - 2026-06-22

First public release.

### Added

- Global hotkey voice input for macOS, with clipboard insertion and automatic `Cmd+V` paste.
- Streaming ASR through Apple SpeechAnalyzer on macOS 26+ and Doubao cloud ASR on macOS 15+.
- Rule-based and LLM post-processing chains, including OpenAI-compatible and Anthropic providers.
- App-aware profiles for selecting ASR, hotwords, and post-processing by frontmost application.
- Live overlay status UI, terminal TUI, history browser, configuration browser, diagnostics, and built-in config templates.
- Optional retained audio in FLAC or AAC, disabled by default.
- launchd service management through `shuo install`, `start`, `stop`, `restart`, `status`, and `uninstall`.
- zh-CN and en-US UI text, with built-in light and dark themes.
