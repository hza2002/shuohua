# Changelog

## 2026-06-17 - Doubao wire protocol corrected to sequenced frames

- Switched Doubao `bigmodel_async` client framing from "no-sequence + LastNoSeq(0x02)" to "PositiveSeq for every frame + NegativeSeq end frame with negated sequence".
- Cross-validated against four independent open-source implementations (`Hypnus-Yuan/doubao-speech`, `chaitin/MonkeyCode`, `Open-Less/openless`, `yyyzl/push-2-talk`); the earlier framing was a defect inherited from a toy reference and forced Doubao's server-side auto-assigned-sequence fallback path, which produced rare 5s+ finalize tails.
- Bumped `default_finalize_timeout_ms` from 5_000 to 12_000 to match openless's measured budget; with the corrected protocol the timeout should virtually never fire.
- Voice layer untouched — VAD-pause finalize is no longer expected to time out, so no soft-failure path was added. If the timeout still fires in the wild, the original "recording marked error" behavior preserves the diagnostic signal.
- Archived `docs/M10.md` and `docs/M10_PLAN.md` (planning + implementation docs for the multi-session ASR work that is now live).

## 2026-06-16 - Logging design transition

- Decided to replace daemon stdout/stderr logging with `tracing` file logs.
- Kept terminal mirroring for foreground `shuo --daemon` development runs.
- Defined official logs as sparse diagnostic logs with strict privacy boundaries.
- Planned monthly history JSONL partitioning without legacy migration.

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
