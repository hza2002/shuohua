# Changelog

本文件只记录公开发布版本的用户可感知变化，最新版本在最上面。

## v0.6.0 - 2026-07-19

### Added

- 新增阿里云百炼（DashScope）实时语音识别 provider（`type = "aliyun"`）：使用 Fun-ASR 实时模型（预设 `fun-asr-realtime`，也可自定义模型），默认中文识别。需自备百炼 API Key。
- History 批量清理：按 [from, to) 时间区间清理记录与音频，支持自定义日期与 180 天预设；清理前预览含语音时长，并需显式 [取消] / [删除] 确认。可只清理音频保留记录，或记录 + 音频一并清理。
- History 分析面板改版：滚动时间窗口、按日期分组的柱状图、可开关的指标。
- 豆包新增可编辑的 `resource_id` 选择（预设 + 自由文本），默认 `2.0 volc.seedasr.sauc.duration`。

### Changed

- 删除文件改为移入系统废纸篓，可恢复：涵盖 History 音频、record + audio、批量清理，以及 Config 的 profile / asr / post 实例文件。移废纸篓失败时记为错误并保留文件，绝不永久删除。
- TUI 新建 ASR 实例与其他配置统一为内存草稿 fill-then-commit（`Ctrl-S` 落盘 / `Esc` 丢弃）。

### Fixed

- 收紧会话 I/O 边界，加固文件删除 / 创建路径。
- History 分析数据惰性加载并缓存范围切换，隐藏分析页无效搜索提示，压缩对齐分析图布局。

## v0.5.0 - 2026-07-07

### Breaking

- Config schema: ASR is now selected by **named instance**. A profile references an ASR config by its file-stem id via `[asr] instance = "<id>"` (renamed from `asr.provider`), and each `asr/<id>.toml` must declare `type = "apple" | "doubao" | "tencent"`. Profiles from 0.4.0 using `provider = "..."` must switch to `[asr] instance = "..."` with a matching typed instance file.
- Config schema: post components are now flat, single files `post/<id>.toml`, each declaring `type = "rule" | "llm"`; `post.chain` and `[post.overrides.<id>]` reference them by that id. Update any post config authored against the previous layout.
- There is no automatic migration. Re-export defaults with `shuo config-template --out <empty-dir>` or edit configs to the new layout; `shuo doctor` flags instances missing a valid `type`.

### Added

- Tencent Cloud realtime ASR provider (`type = "tencent"`): HMAC-SHA1 URL signing, hotwords, and punctuation / number / modal-word filters, with optional server VAD.
- TUI: each config (profile / ASR instance / post component / LLM) is created and edited as a named instance through one unified draft form — field-by-field with validate-on-save and `Ctrl-S` to commit — and the "+ New …" slot lives in the Configure source strip. LLM creation can fetch the provider's model list and test connectivity with `t`.
- TUI: a Profile composer edits a profile as a composition — its ASR instance, per-provider ASR overrides, hotwords, the post chain, and per-LLM-member overrides — with inherited values dimmed, explicit overrides highlighted, and invalid entries (stale overrides, dangling chain members) marked red. `a` adds a chain member, `x` removes the selected one, Shift-J/K reorders, `D` resets an override to inherited, and `X` drops all invalid overrides.
- Profile-level post overrides: `[post.overrides.<id>]` overrides fields of an LLM component in the chain, validated against that component's schema (unknown fields, dangling ids, and rule-component targets are rejected).
- Deleting an ASR instance or post component is reference-checked — blocked while any profile still uses it — and both deletes ask for confirmation.
- Resume hotkey (`hotkey.resume`, default `shift+right_option:double`): after you cancel or an ASR timeout leaves recognized text behind, press it to start a new recording that continues from the previous transcript — the overlay shows the earlier text, and on completion the full post/LLM chain re-runs over the combined text. If there is nothing to resume it starts a normal recording and briefly shows a "new recording" notice so you know the hotkey fired.
- The default microphone preprocessing backend is now `voice.preprocess.backend = "webrtc"` (WebRTC Audio Processing: noise suppression, high-pass filtering, and conservative digital gain), compiled in by default. `off` keeps raw capture and is the only backend that retains the original audio; `apple` remains a fallback for poor audio environments.
- TUI: schema-driven config view with defaults, and in-place editing for config.toml (constrained controls, validate-on-save with rollback, reset-to-default).
- TUI: the footer now lists every shortcut available in the current context (derived from one source), and the mouse works everywhere — click a tab to switch pages, click or scroll the History list, and click config modules/sources/fields. Added `e` to open the selected config file with the default app (`r` still reveals it in Finder).
- TUI: the Configure page now has a detail pane for the selected field showing its full (untruncated) key, value, default, and description with wrapping; long content scrolls with the wheel, PageUp/PageDown, or Ctrl-U/Ctrl-D. The multiline value editor also scrolls to keep long input in view.
- TUI: the History detail pane now has a clickable sub-tab bar (Details/ASR/Pipeline/Sessions/Error/JSON) with the active tab highlighted; `h/l` still cycles the same views. When the detail overflows, scroll it with the wheel over the pane, PageUp/PageDown, or Ctrl-U/Ctrl-D (the wheel over the record list still moves the selection).
- TUI: the Status audio meter is a taller high-resolution braille envelope ("音波"), with a peak/RMS readout and legend below it. Height is the loudness-correlated **RMS** on a fixed dBFS scale (0 dBFS = digital full scale down to a −60 dBFS floor, the conventional level-meter range) — so ambient noise sits low, speech stands tall, and brief transients (high peak, low RMS) don't inflate it. Color independently marks whether the VAD classified each frame as speech. There is no always-on baseline — silence draws nothing (calm) and the meter never auto-ranges, so the display stays honest.

### Changed

- TUI: hotkey fields (`hotkey.trigger`/`cancel`/`resume`) now edit as plain text with the syntax and examples shown in the editor, instead of "press a key to capture". Live capture couldn't express this app's hotkeys (modifier taps, `:double`, left/right sides, F13-F20) and pressing the target key just triggered the running daemon; typing the syntax is validated on save. (This also removes a crash when entering manual mode.)
- TUI: the "new LLM component" wizard no longer auto-opens an editor; edit created files in the TUI, or open them with `e`/`r`.
- TUI: rendering is now event-driven and no longer repaints while idle.
- TUI: the footer status message now auto-clears after a few seconds (errors linger longer) instead of lingering until the next event.
- TUI: the Configure module list no longer shows a raw field count; a red marker appears only when a module has validation errors or missing required fields.
- TUI: empty config values render as `—` so unset fields no longer look blank.
- TUI: the Configure detail pane preserves the value's own line breaks (e.g. a multi-line prompt) instead of collapsing it onto one line.
- TUI: saving a non-`config.toml` file (profile/asr/post) now says "已保存（下次录音生效）" / "Saved (applies next recording)" to make clear only `config.toml` hot-reloads.
- TUI: the Configure "配置来源" bar (horizontal) now switches sources with `h`/`l` and moves focus with `j`/`k` — matching its layout and the History detail tabs, instead of the previous inverted `j`/`k`.

### Fixed

- Canceling a recording now preserves the partial transcript (so the resume hotkey can continue from it) and keeps already-completed pipeline steps in the record.
- TUI: coalesce redraws — a burst of daemon events (e.g. on a VAD speech/silence transition) now repaints the screen once instead of once per event, removing the periodic stutter while recording.
- TUI: `/` starts History search only on the History page, and the `/` footer hint now shows only there (not on Status/Configure, where it does nothing).
- TUI: the multi-line / array value editor now supports arrow-key navigation (Up/Down between lines, Left/Right within a line) and draws the real terminal cursor at the edit position, so editing in the middle of a value is visible and no text shifts as the cursor moves.
- TUI: config select controls (e.g. `openai`/`anthropic`) now cycle with Up/Down as well as Left/Right and `hjkl`.
- Apple voice-processing capture is now a pure VP fallback path without raw handoff/mixing.
- Fixed Apple voice-processing capture lifecycle by rebuilding `AVAudioEngine` for each recording while reusing the helper process, with kill-and-recycle fallback for wedged helpers.
- Fixed microphone downmix to average all input channels instead of only the first, so a multi-channel default input whose first channel is a silent reference no longer produces empty recordings.

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
