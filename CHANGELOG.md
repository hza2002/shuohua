# Changelog

## 2026-06-20 - Release blocker hardening

- Routed UDS `shutdown` through the daemon runtime instead of directly quitting
  AppKit. Active recordings now receive `Stop` and get a bounded shutdown window
  before the overlay main loop is asked to exit.
- Added platform facades for clipboard, autotype, and permission checks, and
  moved macOS-only dependencies into target-specific Cargo dependencies.
- Gated Apple ASR and Darwin hotkey provider compilation to macOS while keeping
  explicit unsupported errors for other platforms.

## 2026-06-19 - Overlay platform boundary refactor

- Split `src/overlay/` into platform-agnostic `command.rs` / `model.rs` /
  `layout.rs` plus a macOS subtree at `overlay/macos/{view,chrome,debug}.rs`.
- Moved Notice / Error TTL, `pending_hide` deferral, and `recording_started`
  duration tracking into `OverlayModel` with `tick(now) -> TickOutcome`. View
  now drives a single fade-out animation via prev/current `visible` comparison.
- Lifted pure layout/text helpers (`display_text_plan`, `live_text_plan`,
  `header_parts`, `format_duration`, `panel_frame`, `first_row_frames`, ...)
  out of view into `overlay/layout.rs`. Geometric functions return
  `LayoutFrame` instead of `NSRect`.
- Split `EffectiveOverlayCfg` into `{ core, macos }` substructs. TOML schema
  breaking change: `[overlay.glass]` removed; `glass_variant`, `glass_style`,
  `subdued`, and `background_blur_radius` now live under `[overlay.macos]`.
  Local `~/.config/shuohua/theme/*.toml` files must be updated manually.
- `main.rs` now can call `overlay::run` without naming the platform; future
  Linux / Windows ports add a sibling `overlay/<platform>/` and a
  `PlatformOverlayCfg` substruct, leaving every other file untouched.

## 2026-06-19 - Voice engine lifecycle hardening

- Treated provider-initiated `AsrEvent::Done` as session completion in both
  modes. VadPause previously left `provider_done = false`, which forced a second
  `send_pcm(is_last=true)` plus another Done wait and surfaced as a spurious
  `asr_timeout` whenever the provider closed a session on its own.
- Added a `RecordingStream::for_test` constructor and split `engine::run` into
  an outer recorder bootstrap and a `run_with_recorder` core. Tests can drive
  the engine end to end without cpal.
- Added `voice/engine_lifecycle_tests.rs` covering Continuous stop/finalize,
  Continuous cancel, ASR stream close, PCM send failure, VadPause Done → Idle,
  VadPause resume open failure, and the multi-session audio-ms invariant.
- Replaced the `Arc<Mutex<Vec<(Vec<i16>, bool)>>>` clippy hot spot in
  `voice::finalize` tests with a `SendPcmCalls` type alias so
  `cargo clippy --all-targets -- -D warnings` is clean.

## 2026-06-19 - Build-generated embedded theme registry

- Made `assets/themes/*.toml` the single source of truth for built-in themes.
- Added compile-time validation for theme IDs, display names, required fields,
  palette references, duplicate names, and deterministic registry generation.
- New valid theme files are embedded into `shuo` and exported by
  `shuo config-template` without editing a Rust registry.

## 2026-06-19 - Voice engine boundary

- Split the recording-time Active/Idle engine from lifecycle completion.
- `voice::engine` now owns PCM routing, session switching, finalize, cancel,
  runtime errors, and retained audio, returning a compact `EngineOutcome`.
- `voice::finish` now contains only the public entry point plus
  post-processing, dispatch, history, and final StateStore/Overlay updates.

## 2026-06-19 - Unified voice recording lifecycle

- Removed the duplicate single-session recording implementation. Continuous
  recording is now a fixed mode of the same lifecycle used by VAD pause/resume.
- Kept Continuous free of Silero, timeline, pre-roll, and Idle state while
  preserving at most one history session.
- Reused one initialization, ASR event, stop/finalize, error/cancel, retained
  audio, post/dispatch, history, StateStore, and Overlay completion path.

## 2026-06-19 - Release history and retained audio formats

- Reset the current history record structure to schema version 1 as the first
  public release baseline; development history is intentionally not migrated.
- Replaced boolean retained-audio configuration with `off`, `lossless` (FLAC),
  and `compact` (AAC-LC 32 kbps). Compact audio measured about 75% smaller than
  FLAC on local voice recordings.
- Added temporary-WAV conversion cleanup, `audio_save` feedback, and TUI lookup
  for the single `.flac` or `.m4a` file associated with a history record ID.

## 2026-06-19 - Voice completion failure handling

- Treat ASR event-stream closure before `Done` and PCM delivery failures during
  normal completion as terminal errors, preserving confirmed segments in error
  history while skipping post-processing and dispatch.
- Surface history append failures through daemon logs, UDS
  `error(kind=history_append)`, and a localized overlay Notice without rolling
  back text that was already delivered.

## 2026-06-17 - Configure refactor foundation

- Moved configuration parsing into a `config/` module tree and added shared
  spec, validation, inventory, and template registry foundations.
- Reworked the TUI Settings tab into Configure modules: Overview, Profile,
  ASR, and Post.
- Integrated Overview doctor output, config file open/reveal actions, manual
  refresh/validate, and daemon `reload_config` from Configure.
- Repaired Configure diagnostics after review: nested TOML path validation,
  full-tree local config diagnostics, manual-only TUI doctor trigger, safer
  editor/Finder actions, and explicit `doctor --runtime` separation.
- Added generated config templates via `shuo config-template` and a first LLM
  post component wizard that writes `post/llm/<file_id>.toml` without
  auto-attaching it to profile chains.

## 2026-06-17 - First TUI status/history audio pass

- Added the first usable TUI retained-audio workflow. This initially used
  `state_dir()/audio/<recording_id>.wav`; the release format was later changed
  to one retained `.flac` or `.m4a` file per recording, with `.tmp.wav` kept
  only as a conversion intermediate.
- Reworked History presentation with colored stats/list/details, page-specific footer shortcuts, inline audio metadata in `History details`, and minimal zh-CN/en-US i18n coverage with key-alignment tests.
- Made Status meter width responsive and aligned TUI rendering to the 50ms audio meter cadence while draining IPC/key events between frames, fixing the previous audio-meter backlog that could fill the IPC client queue.
- Added 1s throttling for `IPC client queue full` warnings so abnormal client backpressure stays diagnosable without flooding logs.
- Observed that occasional Recording → Idle pauses can still happen before the Idle UI update because VAD pause must finalize the current ASR session first; logs/traces point to provider finalize/open tail latency rather than TUI rendering.

## 2026-06-17 - Doubao wire protocol corrected to sequenced frames

- Switched Doubao `bigmodel_async` client framing from "no-sequence + LastNoSeq(0x02)" to "PositiveSeq for every frame + NegativeSeq end frame with negated sequence".
- Cross-validated against four independent open-source implementations (`Hypnus-Yuan/doubao-speech`, `chaitin/MonkeyCode`, `Open-Less/openless`, `yyyzl/push-2-talk`); the earlier framing was a defect inherited from a toy reference and forced Doubao's server-side auto-assigned-sequence fallback path, which produced rare 5s+ finalize tails.
- Bumped `default_finalize_timeout_ms` from 5_000 to 12_000 to match openless's measured budget; with the corrected protocol the timeout should virtually never fire.
- Voice layer untouched — VAD-pause finalize is no longer expected to time out, so no soft-failure path was added. If the timeout still fires in the wild, the original "recording marked error" behavior preserves the diagnostic signal.
- Archived the multi-session ASR planning and implementation notes now that the feature is live.

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

## Earlier changes

- Built the single-process daemon + TUI architecture with UDS state fanout and append-only history.
- Implemented global hotkey handling, recorder lifecycle, overlay feedback, clipboard dispatch, and launchd integration.
- Added Doubao streaming ASR as the first production provider.
- Added per-app profiles, post-processing chains, LLM cleanup, and overlay notice/error feedback.
