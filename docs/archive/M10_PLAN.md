# M10 Multi-session ASR Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let one recording contain multiple ASR provider sessions controlled by local Silero VAD, reducing paid silent audio without changing history schema or final user-facing text behavior.

**Architecture:** Keep `AsrProvider` / `AsrSession` unchanged. Voice owns VAD, sample timeline, replay buffer, and multi-session orchestration; providers only expose private config values to the caller at construction time. History remains one record per recording, with `asr.sessions[]` populated from finalized provider sessions.

**Tech Stack:** Rust, tokio, cpal PCM at 16kHz s16le mono, existing ASR traits, optional `voice_activity_detector` crate for Silero VAD, existing JSONL history schema v2.

---

## Scope

Build:

- `[voice.vad]` config parsing for VAD backend and timing parameters.
- Provider private config fields `idle_pause` and `finalize_timeout_ms`.
- A voice-visible provider runtime policy without changing `AsrProvider`.
- Sample-indexed PCM timeline and replay ring buffer inside voice.
- Multi-session controller for `Active -> Pausing -> Idle -> Opening -> Active`.
- History writer input that can emit multiple `asr.sessions[]`.
- Tests for single-session compatibility, pause/resume, overlap, timeout, open failure, and history semantics.

Do not build:

- No history schema version bump.
- No UDS protocol change.
- No ASR trait change.
- No Apple SpeechDetector VAD.
- No multi-VAD voting or hybrid RMS fallback.
- No cross-session text dedup in M10.
- No release history fields for partials, VAD frames, or debug events.

## File Map

- Modify `src/config.rs`: add `VoiceVadCfg`, defaults, serde parsing tests.
- Modify `src/main.rs`: construct provider runtime, pass VAD/provider policy into `SessionParams`.
- Modify `src/asr/providers/doubao.rs`: parse `idle_pause` and `finalize_timeout_ms`.
- Modify `src/asr/providers/apple.rs`: parse the same fields, default disabled.
- Modify `src/voice/vad.rs`: expose policy conversion from config and keep pure controller tests.
- Create `src/voice/silero.rs`: formal Silero backend wrapper behind the existing optional dependency.
- Create `src/voice/timeline.rs`: sample-indexed PCM chunk/ring-buffer helpers.
- Modify `src/voice/trace.rs`: use configured VAD values instead of hard-coded constants where available.
- Modify `src/voice/finish.rs`: extract provider-session finalization, add multi-session path, keep single-session path.
- Modify `src/voice/mod.rs`: export new voice modules.
- Modify `docs/M10.md`, `docs/DESIGN.md`, `docs/MODULES.md`, `docs/SCHEMA.md`, `docs/CLI.md`: keep implementation docs current after code lands.

## Invariants

- Default config keeps current behavior: one recording opens one ASR session.
- `idle_pause = false` for the selected provider must bypass all multi-session control and preserve current history output.
- `voice.vad.backend = "off"` must bypass local VAD even if provider `idle_pause = true`.
- `voice.vad.backend = "silero"` is the only M10 active backend.
- `sessions[]` are ordered by `started_at`; adjacent sessions may overlap by at most `max_overlap_ms`.
- `sessions[].audio_ms = ended_at - started_at` on the recording timeline.
- `asr.audio_ms = sum(sessions[].audio_ms)`.
- On provider finalize timeout during pause, first M10 release marks the whole recording error and does not open another session.

---

## Task 1: Voice VAD Config

**Files:**

- Modify: `src/config.rs`
- Modify: `docs/DESIGN.md`
- Test: existing `config::tests`

- [ ] **Step 1: Add failing config tests**

Add tests in `src/config.rs`:

```rust
#[test]
fn voice_vad_defaults_are_disabled() {
    let cfg: Config = toml::from_str(
        r#"
[hotkey]
trigger = "f16"
"#,
    )
    .unwrap();

    assert_eq!(cfg.voice.vad.backend, VoiceVadBackend::Off);
    assert_eq!(cfg.voice.vad.threshold, 0.5);
    assert_eq!(cfg.voice.vad.pause_silence_ms, 1500);
    assert_eq!(cfg.voice.vad.pre_roll_ms, 300);
    assert_eq!(cfg.voice.vad.max_overlap_ms, 200);
    assert_eq!(cfg.voice.vad.min_start_voiced_frames, 2);
}

#[test]
fn voice_vad_can_parse_silero_settings() {
    let cfg: Config = toml::from_str(
        r#"
[hotkey]
trigger = "f16"

[voice.vad]
backend = "silero"
threshold = 0.42
pause_silence_ms = 1200
pre_roll_ms = 250
max_overlap_ms = 180
min_start_voiced_frames = 3
"#,
    )
    .unwrap();

    assert_eq!(cfg.voice.vad.backend, VoiceVadBackend::Silero);
    assert_eq!(cfg.voice.vad.threshold, 0.42);
    assert_eq!(cfg.voice.vad.pause_silence_ms, 1200);
    assert_eq!(cfg.voice.vad.pre_roll_ms, 250);
    assert_eq!(cfg.voice.vad.max_overlap_ms, 180);
    assert_eq!(cfg.voice.vad.min_start_voiced_frames, 3);
}
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
cargo test config::tests::voice_vad_defaults_are_disabled config::tests::voice_vad_can_parse_silero_settings
```

Expected: FAIL because `VoiceVadBackend` and `voice.vad` do not exist.

- [ ] **Step 3: Implement minimal config types**

Add to `src/config.rs`:

```rust
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VoiceVadBackend {
    Off,
    Silero,
}

fn default_vad_backend() -> VoiceVadBackend {
    VoiceVadBackend::Off
}
fn default_vad_threshold() -> f32 {
    0.5
}
fn default_pause_silence_ms() -> u32 {
    1500
}
fn default_pre_roll_ms() -> u32 {
    300
}
fn default_max_overlap_ms() -> u32 {
    200
}
fn default_min_start_voiced_frames() -> u32 {
    2
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct VoiceVadCfg {
    #[serde(default = "default_vad_backend")]
    pub backend: VoiceVadBackend,
    #[serde(default = "default_vad_threshold")]
    pub threshold: f32,
    #[serde(default = "default_pause_silence_ms")]
    pub pause_silence_ms: u32,
    #[serde(default = "default_pre_roll_ms")]
    pub pre_roll_ms: u32,
    #[serde(default = "default_max_overlap_ms")]
    pub max_overlap_ms: u32,
    #[serde(default = "default_min_start_voiced_frames")]
    pub min_start_voiced_frames: u32,
}

impl Default for VoiceVadCfg {
    fn default() -> Self {
        Self {
            backend: default_vad_backend(),
            threshold: default_vad_threshold(),
            pause_silence_ms: default_pause_silence_ms(),
            pre_roll_ms: default_pre_roll_ms(),
            max_overlap_ms: default_max_overlap_ms(),
            min_start_voiced_frames: default_min_start_voiced_frames(),
        }
    }
}
```

Add to `VoiceCfg`:

```rust
#[serde(default)]
pub vad: VoiceVadCfg,
```

Add to `VoiceCfg::default()`:

```rust
vad: VoiceVadCfg::default(),
```

- [ ] **Step 4: Run focused and full config tests**

Run:

```bash
cargo test config::tests
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs docs/DESIGN.md
git commit -m "Add voice VAD configuration"
```

---

## Task 2: Provider Idle-Pause Runtime Policy

**Files:**

- Modify: `src/asr/providers/doubao.rs`
- Modify: `src/asr/providers/apple.rs`
- Modify: `src/main.rs`
- Modify: `src/voice/finish.rs`
- Test: provider config tests in `doubao.rs` and `apple.rs`

- [ ] **Step 1: Add provider config tests**

In `src/asr/providers/doubao.rs`, add a test that deserializes:

```rust
let cfg: DoubaoConfig = toml::from_str(
    r#"
app_key = "ak"
access_key = "sk"
idle_pause = true
finalize_timeout_ms = 7000
"#,
)
.unwrap();
assert!(cfg.idle_pause);
assert_eq!(cfg.finalize_timeout_ms, 7000);
```

In `src/asr/providers/apple.rs`, add:

```rust
let cfg: AppleConfig = toml::from_str(
    r#"
idle_pause = true
finalize_timeout_ms = 3000
"#,
)
.unwrap();
assert!(cfg.idle_pause);
assert_eq!(cfg.finalize_timeout_ms, 3000);

let default = AppleConfig::default();
assert!(!default.idle_pause);
assert_eq!(default.finalize_timeout_ms, 5000);
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
cargo test asr::providers::doubao::tests:: asr::providers::apple::tests::
```

Expected: FAIL because config fields do not exist.

- [ ] **Step 3: Add provider config fields**

Add to both config structs:

```rust
#[serde(default)]
pub idle_pause: bool,
#[serde(default = "default_finalize_timeout_ms")]
pub finalize_timeout_ms: u64,
```

Add helper in each file:

```rust
fn default_finalize_timeout_ms() -> u64 {
    5000
}
```

Update `AppleConfig::default()`.

- [ ] **Step 4: Add voice-visible runtime policy without changing ASR trait**

In `src/main.rs`, add:

```rust
struct ProviderRuntime {
    provider: Arc<dyn asr::AsrProvider>,
    idle_pause: bool,
    finalize_timeout_ms: u64,
}
```

Change `build_provider()` to return `Result<ProviderRuntime>`. For each provider, construct the concrete provider first, read its public config values through an inherent method, then store the trait object.

Add inherent methods to providers:

```rust
impl DoubaoProvider {
    pub fn idle_pause(&self) -> bool {
        self.config.idle_pause
    }

    pub fn finalize_timeout_ms(&self) -> u64 {
        self.config.finalize_timeout_ms
    }
}
```

Repeat for `AppleProvider`.

Pass `provider_runtime.idle_pause` and `provider_runtime.finalize_timeout_ms` into `SessionParams`.

- [ ] **Step 5: Add fields to `SessionParams`**

In `src/voice/finish.rs`:

```rust
pub struct SessionParams {
    pub auto_paste: bool,
    pub record_audio: bool,
    pub vad_trace: bool,
    pub idle_pause: bool,
    pub finalize_timeout_ms: u64,
    pub stop_delay_ms: u32,
    ...
}
```

Use `params.finalize_timeout_ms` in the existing final wait instead of `Duration::from_secs(5)`.

- [ ] **Step 6: Run verification**

Run:

```bash
cargo fmt
cargo test asr::providers::doubao::tests:: asr::providers::apple::tests:: config::tests
cargo check
```

Expected: all PASS; existing unused warnings may remain.

- [ ] **Step 7: Commit**

```bash
git add src/asr/providers/doubao.rs src/asr/providers/apple.rs src/main.rs src/voice/finish.rs
git commit -m "Add provider idle pause policy"
```

---

## Task 3: Timeline Ring Buffer

**Files:**

- Create: `src/voice/timeline.rs`
- Modify: `src/voice/mod.rs`
- Test: `voice::timeline::tests`

- [ ] **Step 1: Write ring buffer tests**

Create tests for:

- `push()` assigns monotonically increasing sample ranges.
- `slice_from(start_sample)` returns available samples from the ring.
- `slice_from()` clamps to the oldest retained sample.
- Retention keeps at least `pre_roll_ms + max_overlap_ms`.

Use sample-rate constant `16_000`.

- [ ] **Step 2: Implement data types**

Create `src/voice/timeline.rs`:

```rust
const SAMPLE_RATE: u64 = 16_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmChunk {
    pub start_sample: u64,
    pub samples: Vec<i16>,
}

#[derive(Debug, Clone)]
pub struct PcmTimeline {
    next_sample: u64,
    retained_start: u64,
    retained: Vec<i16>,
    max_retained_samples: usize,
}
```

Implement:

```rust
impl PcmTimeline {
    pub fn new(max_retained_ms: u32) -> Self;
    pub fn push(&mut self, samples: &[i16]) -> PcmChunk;
    pub fn next_sample(&self) -> u64;
    pub fn oldest_sample(&self) -> u64;
    pub fn slice_from(&self, start_sample: u64) -> PcmChunk;
}

pub fn ms_to_samples(ms: u32) -> u64;
pub fn samples_to_ms(samples: u64) -> u64;
```

- [ ] **Step 3: Export module**

In `src/voice/mod.rs`:

```rust
pub mod timeline;
```

- [ ] **Step 4: Run tests**

```bash
cargo test voice::timeline::tests
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/voice/timeline.rs src/voice/mod.rs
git commit -m "Add PCM timeline ring buffer"
```

---

## Task 4: Formal Silero Backend

**Files:**

- Create: `src/voice/silero.rs`
- Modify: `src/voice/mod.rs`
- Modify: `src/voice/trace.rs`
- Test: `voice::silero::tests`

- [ ] **Step 1: Define backend wrapper behind feature**

Create `src/voice/silero.rs` with:

```rust
#[derive(Debug, Clone, Copy)]
pub struct SileroConfig {
    pub threshold: f32,
}

#[cfg(feature = "dev-vad-trace")]
pub struct SileroVad {
    detector: voice_activity_detector::VoiceActivityDetector,
    threshold: f32,
    buffer: Vec<i16>,
    sample_offset: u64,
}
```

Expose:

```rust
#[cfg(feature = "dev-vad-trace")]
impl SileroVad {
    pub fn new(config: SileroConfig) -> anyhow::Result<Self>;
    pub fn accept(&mut self, samples: &[i16]) -> Vec<(u64, crate::voice::vad::VadFrame)>;
}
```

The returned `u64` is `frame_start_sample`.

- [ ] **Step 2: Keep default build compiling**

Add non-feature stubs:

```rust
#[cfg(not(feature = "dev-vad-trace"))]
pub struct SileroVad;
```

Do not expose a fake working constructor in default builds. M10 runtime must only instantiate Silero when the dependency is compiled.

- [ ] **Step 3: Refactor trace to use `SileroVad`**

Replace the hard-coded detector in `src/voice/trace.rs` with the shared backend so trace and runtime use the same frame boundaries.

- [ ] **Step 4: Export module**

In `src/voice/mod.rs`:

```rust
pub mod silero;
```

- [ ] **Step 5: Run verification**

```bash
cargo check
cargo test --features dev-vad-trace,dev-vad-probe voice::trace::tests
cargo check --features dev-vad-trace,dev-vad-probe
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/voice/silero.rs src/voice/trace.rs src/voice/mod.rs
git commit -m "Extract Silero VAD backend"
```

---

## Task 5: Extract Provider Session Finalization

**Files:**

- Modify: `src/voice/finish.rs`
- Test: existing `voice::finish::tests`, new fake event tests if needed

- [ ] **Step 1: Extract helper signature**

In `src/voice/finish.rs`, extract the existing final wait loop into an internal helper:

```rust
async fn finalize_provider_session(
    session: &mut Box<dyn AsrSession>,
    events: &mut mpsc::Receiver<AsrEvent>,
    pending_segments: &mut Vec<SegmentCapture>,
    finalize_timeout_ms: u64,
    trace: &mut TraceRecorder,
    recording_started_instant: Instant,
    state: &StateStore,
    recording_id: &str,
) -> Result<FinalizeOutcome, HistoryError>
```

Define:

```rust
enum FinalizeOutcome {
    Done,
    Canceled,
}
```

Keep the current stop path behavior identical.

- [ ] **Step 2: Preserve timeout semantics**

The helper must:

- call `session.send_pcm(&[], true)`;
- keep collecting `Segment` and `Partial`;
- return `Err(HistoryError { kind: "asr_timeout", msg: "timeout waiting final" })` on timeout;
- return `Err(HistoryError { kind: "asr_send_last", ... })` if the last frame cannot be sent.

- [ ] **Step 3: Run compatibility tests**

```bash
cargo test voice::finish::tests
cargo test
```

Expected: existing single-session tests still PASS.

- [ ] **Step 4: Commit**

```bash
git add src/voice/finish.rs
git commit -m "Extract ASR session finalization"
```

---

## Task 6: Multi-session Capture Model

**Files:**

- Modify: `src/voice/finish.rs`
- Test: `voice::finish::tests`

- [ ] **Step 1: Add internal session capture type**

Add:

```rust
struct SessionCapture {
    started_at: Instant,
    ended_at: Instant,
    audio_samples: u64,
    segments: Vec<SegmentCapture>,
}
```

Add helper:

```rust
fn session_text(segments: &[SegmentCapture]) -> String {
    segments.iter().map(|s| s.text.as_str()).collect()
}
```

- [ ] **Step 2: Change history folding to accept multiple sessions**

Replace the current single-session fold with a helper:

```rust
fn build_asr_sessions(
    sessions: &[SessionCapture],
    recording_started_at: time::OffsetDateTime,
    recording_started_instant: Instant,
) -> Vec<AsrSessionHistory>
```

It must convert each session timeline into `started_at`, `ended_at`, `audio_ms`, and `text`.

- [ ] **Step 3: Add unit tests**

Add tests that verify:

- two captured sessions produce two history sessions;
- `asr.audio_ms` equals the sum;
- overlapping session instants are preserved instead of rejected.

- [ ] **Step 4: Keep current path using one capture**

Before enabling VAD, current single-session code should create exactly one `SessionCapture`.

- [ ] **Step 5: Run tests**

```bash
cargo test voice::finish::tests
cargo test state::history::tests
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/voice/finish.rs
git commit -m "Support multi-session history capture"
```

---

## Task 7: Multi-session Controller Path

**Files:**

- Modify: `src/voice/finish.rs`
- Modify: `src/voice/vad.rs`
- Modify: `src/voice/timeline.rs`
- Test: `voice::finish::tests`, `voice::vad::tests`, `voice::timeline::tests`

- [ ] **Step 1: Add controller mode switch**

At the start of `run_recording()`:

```rust
let multi_session_enabled =
    params.idle_pause && matches!(params.vad.backend, VoiceVadBackend::Silero);
```

If false, call the current single-session flow.

If true, call a new internal function:

```rust
async fn run_multi_session_recording(...)
```

- [ ] **Step 2: Add state enum**

Inside `finish.rs`:

```rust
enum MultiSessionState {
    Active,
    Pausing,
    Idle,
    Opening,
}
```

Do not expose it outside voice.

- [ ] **Step 3: Implement Active**

In Active:

- receive PCM;
- push to `PcmTimeline`;
- send current chunk to current ASR session;
- feed chunk to Silero;
- on `VadTransition::SilenceStarted`, enter Pausing.

- [ ] **Step 4: Implement Pausing**

In Pausing:

- call `finalize_provider_session()`;
- store `SessionCapture`;
- close current session;
- enter Idle.

On finalize error, finish the whole recording with error.

- [ ] **Step 5: Implement Idle**

In Idle:

- keep receiving PCM;
- push it to timeline;
- feed it to Silero;
- do not send PCM to ASR;
- on `VadTransition::SpeechStarted`, compute replay range and enter Opening.

- [ ] **Step 6: Implement Opening**

Use:

```text
desired_start = speech_start_sample - pre_roll_samples
send_start = max(desired_start, last_sent_sample - max_overlap_samples)
```

Clamp `send_start` to `timeline.oldest_sample()`. Open a new provider session, send `timeline.slice_from(send_start)`, then return to Active.

- [ ] **Step 7: Stop handling**

On user Stop:

- if Active, drain `stop_delay_ms`, finalize current session, then post/dispatch/write one history record;
- if Idle, skip ASR finalize and post/dispatch/write the sessions already captured;
- if Pausing/Opening, complete or error according to the current operation result.

- [ ] **Step 8: Add fake-provider tests**

Add tests for:

- silence after speech creates two sessions when speech resumes;
- short silence below `pause_silence_ms` keeps one session;
- resume sends pre-roll samples;
- resume overlap does not exceed `max_overlap_ms`;
- open failure after Idle marks recording error;
- finalize timeout marks recording error.

- [ ] **Step 9: Run verification**

```bash
cargo fmt
cargo test voice::vad::tests voice::timeline::tests voice::finish::tests
cargo test --features dev-vad-trace,dev-vad-probe
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add src/voice/finish.rs src/voice/vad.rs src/voice/timeline.rs
git commit -m "Add multi-session ASR controller"
```

---

## Task 8: Trace and Documentation Alignment

**Files:**

- Modify: `src/voice/trace.rs`
- Modify: `docs/M10.md`
- Modify: `docs/SCHEMA.md`
- Modify: `docs/MODULES.md`
- Modify: `docs/CLI.md`
- Test: trace tests

- [ ] **Step 1: Add trace events for real session boundaries**

When M10 is enabled, trace must include:

```json
{"event":"session_start","session_index":0,"start_ms":0}
{"event":"session_finalize_start","session_index":0,"t_ms":1234}
{"event":"session_done","session_index":0,"start_ms":0,"end_ms":1534,"audio_ms":1534}
{"event":"session_open_error","session_index":1,"t_ms":2000,"message":"..."}
```

Keep existing `vad_frame`, `vad_transition`, `asr_segment`, and `recording_end`.

- [ ] **Step 2: Add trace tests**

Extend `voice::trace::tests::writes_jsonl_trace_events_and_summary` or add a new test to assert session boundary events serialize as JSONL.

- [ ] **Step 3: Update docs**

Update:

- `docs/M10.md`: mark implementation status and defaults.
- `docs/SCHEMA.md`: ensure `audio_ms` and overlap semantics match code.
- `docs/MODULES.md`: mark `voice/silero.rs` and `voice/timeline.rs`.
- `docs/CLI.md`: document how to run dev trace after M10.

- [ ] **Step 4: Run docs-related verification**

```bash
rg -n 'superpowers/(specs|plans)' docs
rg -n 'supports_idle_''pause' docs
cargo test --features dev-vad-trace,dev-vad-probe voice::trace::tests
```

Expected: `rg` returns no stale superpowers links or old capability API names in tracked docs; tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/voice/trace.rs docs/M10.md docs/SCHEMA.md docs/MODULES.md docs/CLI.md
git commit -m "Document M10 session tracing"
```

---

## Task 9: Final Verification and Manual Acceptance

**Files:**

- Modify only if verification reveals a bug.

- [ ] **Step 1: Run full automated verification**

```bash
cargo fmt --check
cargo check
cargo check --features dev-vad-trace,dev-vad-probe
cargo test
cargo test --features dev-vad-trace,dev-vad-probe
```

Expected: all PASS; existing warnings are acceptable only if they predate M10 and are not worsened.

- [ ] **Step 2: Manual recording check with idle pause disabled**

Use current normal config:

```toml
[voice.vad]
backend = "off"
```

or provider `idle_pause = false`.

Expected:

- one recording creates one history record;
- `asr.sessions.len() == 1`;
- final text behavior matches pre-M10.

- [ ] **Step 3: Manual recording check with Doubao idle pause enabled**

Use:

```toml
[voice.vad]
backend = "silero"
threshold = 0.5
pause_silence_ms = 1500
pre_roll_ms = 300
max_overlap_ms = 200
min_start_voiced_frames = 2
```

and in `asr/doubao.toml`:

```toml
idle_pause = true
finalize_timeout_ms = 5000
```

Expected:

- long silence creates multiple `asr.sessions[]`;
- no obvious missing first/last words;
- no obvious duplicated phrase caused by overlap;
- `asr.audio_ms` is lower than recording duration on recordings with real pauses, except small overlap cost;
- trace shows VAD active intervals covering ASR segment intervals.

- [ ] **Step 4: Manual Apple check**

Use Apple provider with `idle_pause = false`.

Expected:

- Apple remains single-session by default;
- no local VAD dependency is required for Apple default use.

- [ ] **Step 5: Commit final docs or fixes**

If docs or small fixes changed:

```bash
git add <changed-files>
git commit -m "Verify M10 multi-session ASR"
```

---

## Rollback

Rollback must be possible without data migration:

- Disable runtime behavior by setting provider `idle_pause = false`.
- Disable VAD by setting `[voice.vad] backend = "off"`.
- Revert M10 code commits if needed; history v2 remains readable because schema fields did not change.
- Existing single-session history records remain valid because `sessions[]` with one item is still the common case.

## Acceptance Criteria

- Default configuration preserves current one-session behavior.
- Doubao with `idle_pause = true` can create multiple provider sessions in one recording.
- Final history remains one JSONL record per recording.
- `asr.sessions[]` contains one entry per provider session.
- M10 does not require `AsrProvider` / `AsrSession` trait changes.
- M10 does not add VAD debug fields to history.
- Automated tests pass with and without dev VAD features.
- Manual Doubao recordings show no missed speech in the tested samples.
