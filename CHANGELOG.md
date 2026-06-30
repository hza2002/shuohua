# Changelog

本文件只记录公开发布版本的用户可感知变化，最新版本在最上面。

## v0.4.0 - 2026-06-30

### Added

- Added `shuo report` to generate a safe support bundle with summaries, diagnostics, and redacted logs without including config, history, or retained audio.

### Changed

- Standardized the supported install path on `~/.local/bin/shuo`; `shuo update` now writes there without sudo and reports migration guidance when another binary path is in use.
- Improved `shuo doctor` and `shuo service status` install drift diagnostics for the running binary, launchd plist binary, and first `shuo` on `PATH`.

### Fixed

- Fixed service start/install so launchd success is not reported until the daemon is reachable and stable.
- Fixed service stop so an already stopped daemon prints `daemon: 未运行` and exits successfully.
- Fixed daemon IPC absence handling so only missing/refused sockets are treated as not running; other IPC errors are surfaced.

## v0.3.0 - 2026-06-30

### Added

- Added macOS system voice-processing capture as the default microphone preprocessing path, improving gain, echo handling, and background-noise treatment.
- Added `[voice.preprocess].backend` with `apple` and `off` options, and updated generated config templates and diagnostics to expose the effective setting.
- Added `shuo doctor --apple-capture-smoke` to run a short real microphone capture check for the Apple voice-processing path.
- Added overlay profile switching for the current app, allowing users to bind the frontmost app to another profile from the recording overlay for future sessions.

### Changed

- Open microphone capture and ASR sessions in parallel to reduce recording startup latency.
- Reuse the Apple capture helper across sessions and keep helper startup/lifecycle timing in daemon logs for easier diagnosis.
- Improved overlay transcript rendering with measured AppKit text layout, internal scrolling for long ASR text, clearer scroll affordances, and more stable profile picker sizing.
- Release notes now embed the matching `CHANGELOG.md` section into the GitHub Release body.

### Fixed

- Fixed Apple capture helper lifecycle issues including bounded startup waits, daemon-owned helper lifetime, converter drain on stop, and protocol hardening.
- Fixed voice startup and cancel paths so capture or ASR startup failures do not wait on the other side indefinitely.
- Fixed VAD pause sessions so recording starts when speech is detected rather than too early.
- Fixed finalize behavior so the final ASR send is covered by the finalize timeout and ASR partials remain visible in the overlay after recording stops.
- Fixed empty or contentless recordings so they do not create misleading history records or run post-processing.
- Fixed overlay routing and display issues around profile ids, chain summaries, body scrolling, tail geometry, and text sizing.

## v0.2.0 - 2026-06-24

### Added

- Added shell completion generation through `shuo completions`, covering the
  common shells documented by the CLI.
- Added `shuo update` so installed binaries can update from GitHub release
  artifacts with checksum verification.
- Added release archive helpers and updated the release workflow so published
  artifacts include the binary, checksums, license, and bilingual README files.

### Changed

- Moved launchd lifecycle operations under `shuo service`, with refreshed
  command text, documentation, and localized messages for the new command
  layout.
- Improved overlay glass frame synchronization so the visual effect frame stays
  aligned with overlay layout changes.
- Removed the unused overlay thinking-delay setting from configuration schema,
  generated templates, themes, and localization.

### Fixed

- Fixed a VadPause stop edge case where pressing the trigger while ASR finalize
  was transitioning back to idle could be swallowed, leaving the session stuck
  until cancel.

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
