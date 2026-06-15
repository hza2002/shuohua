# Changelog

## 2026-06-15 - Local Apple ASR baseline

- Added `apple` ASR provider backed by macOS 26 SpeechAnalyzer through an embedded Swift helper.
- Validated real Chinese, English-mixed, and technical-term dictation against user recordings.
- Set `apple` as the privacy-first local provider for daily testing.
- Kept `doubao` as an available cloud provider, but no longer plan another local model fallback while Apple quality remains acceptable.
- Reverted the earlier Whisper.cpp experiment because Whisper's batch/windowed recognition model did not fit real-time dictation.
- Removed obsolete prototypes, examples, and internal spec drafts from the repository so the next phase starts from a clean tree.

Security note: the Apple provider uses Apple's on-device SpeechAnalyzer assets after installation. `shuohua` does not send audio to third-party ASR services when `provider = "apple"` is selected; the remaining trust boundary is the local macOS Speech framework and Apple's asset installation path.

## Earlier milestones

- Built the single-process daemon + TUI architecture with UDS state fanout and append-only history.
- Implemented global hotkey handling, recorder lifecycle, overlay feedback, clipboard dispatch, and launchd integration.
- Added Doubao streaming ASR as the first production provider.
- Added per-app profiles, post-processing chains, LLM cleanup, and overlay notice/error feedback.
