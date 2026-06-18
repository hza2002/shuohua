# Config Field Descriptions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make config field descriptions a single source of truth consumed by template generation and TUI Configure.

**Architecture:** `FieldSpec` stores stable metadata, including a `description_key`. Domain specs move behind `config::schema`; template rendering, diagnostics, and TUI rows consume that registry instead of maintaining parallel field definitions.

**Tech Stack:** Rust, TOML, existing `assets/i18n/*.toml`, existing unit tests.

---

### Task 1: Extend FieldSpec Metadata

**Files:**
- Modify: `src/config/spec.rs`

- [ ] Add `description_key: Option<&'static str>` to `FieldSpec`.
- [ ] Add builder/accessor methods: `description_key(...)` and `description_key_value()`.
- [ ] Keep validation behavior unchanged.
- [ ] Add a unit test that confirms description keys survive builder chaining.

### Task 2: Add Config Schema Registry

**Files:**
- Create: `src/config/schema.rs`
- Modify: `src/config/mod.rs`
- Modify: `src/config/template.rs`
- Modify: `src/config/diagnostics.rs`

- [ ] Move shared specs for main, profile, ASR Apple, ASR Doubao, post rule, and post LLM into `config::schema`.
- [ ] Attach description keys to fields that appear in generated starter templates.
- [ ] Replace duplicate specs in `template.rs` and `diagnostics.rs` with calls to `schema`.
- [ ] Keep provider-specific ownership in `config`; do not move parsing out of its current modules.

### Task 3: Render Template Comments From Schema

**Files:**
- Modify: `src/config/template.rs`
- Modify: `src/cli/config_template.rs`
- Modify: `src/cli/mod.rs`
- Modify: `src/main.rs`
- Modify: `assets/i18n/en-US.toml`
- Modify: `assets/i18n/zh-CN.toml`

- [ ] Change template rendering to resolve `FieldSpec.description_key` into comments before each rendered field.
- [ ] Add `--lang <auto|en-US|zh-CN>` to `shuo config-template`.
- [ ] Use configured `ui.language` when `--lang auto`; fall back to `en-US`.
- [ ] Add i18n entries for all description keys used by generated templates.

### Task 4: Expose Descriptions To TUI Configure

**Files:**
- Modify: `src/tui/settings.rs`
- Modify: `src/tui/panes.rs`

- [ ] Add a `description` field to `SettingsRow`.
- [ ] Populate it from `config::schema` where a row maps to a known field.
- [ ] Render the selected row description in the Configure detail/help area without translating config keys.

### Task 5: Verification

**Files:**
- Modify tests in touched files as needed.

- [ ] Add tests that generated templates contain field comments and remain valid TOML.
- [ ] Add tests that `--lang zh-CN` generates Chinese comments.
- [ ] Run `cargo fmt`.
- [ ] Run `cargo check`.
- [ ] Run `cargo test`.
- [ ] Run `cargo run -- config-template --out /tmp/shuohua-config-template-i18n --lang zh-CN` and inspect `config.toml`.
