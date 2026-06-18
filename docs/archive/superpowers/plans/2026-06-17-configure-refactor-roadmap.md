# Configure Refactor Roadmap

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the configuration stack around a single `config` module as the source of truth, then grow TUI Configure into a module-based configuration manager with diagnostics, editor launchers, and template-driven component creation.

**Architecture:** Keep runtime config parsing, inventory, validation, diagnostics, and template generation in one `config` module tree. TUI reads only structured inventories and diagnostics; it does not parse ad hoc TOML summaries. Provider/post/profile-specific config remains file-based, but every read path, validator, and template is derived from the same schema/spec registry.

**Tech Stack:** Rust, `serde`, `toml`, `anyhow`, `tokio`, `ratatui`, existing i18n loader, existing IPC client/server.

---

## Program Shape

This is a multi-session refactor program. Do not try to finish it in one session.

Execution rule:

- Each phase ends with `cargo fmt`, `cargo check`, `cargo test`.
- Each phase ends with `git status --short --branch -uall`.
- Each phase is committed separately.
- After each phase, stop and wait for user review before starting the next phase.

Main design constraints:

- Config files remain the source of truth.
- The TUI is a configuration manager, not a TOML form editor.
- Unknown fields warn by default; missing required fields and type mismatches are errors.
- Official templates are generated from the same spec data that powers validation.
- External edits are allowed at any time; TUI must refresh from disk instead of holding writable state.

---

## File Structure

The refactor should converge on these responsibilities:

- `src/config/mod.rs` - public entry point; existing `Config` API stays stable where possible
- `src/config/spec.rs` - field/spec metadata, severity, diagnostics, common validation helpers
- `src/config/inventory.rs` - structured scan of all config files for TUI and doctor
- `src/config/template.rs` - official template registry and rendering
- `src/config/main.rs` - top-level `config.toml` schema and helpers
- `src/config/profile.rs` - `profile/*.toml` schema and inventory
- `src/config/post/mod.rs` - post-processor config namespace
- `src/config/post/rule.rs` - rule processor schema
- `src/config/post/llm.rs` - LLM processor schema and templates
- `src/config/asr/mod.rs` - ASR provider config namespace
- `src/config/asr/apple.rs` - Apple provider schema and config loading
- `src/config/asr/doubao.rs` - Doubao provider schema and config loading
- `src/config/theme.rs` - theme namespace placeholder, later expanded
- `src/cli/doctor.rs` - consume structured config validation / diagnostics
- `src/tui/settings.rs` - replace ad hoc row scanning with structured inventory
- `src/tui/panes.rs` - Configure module UI
- `src/tui/mod.rs` - app state for Configure navigation / refresh / diagnostics
- `assets/config/**` - official templates and manifest
- `assets/i18n/*.toml` - UI labels for Configure modules, diagnostics, templates, and editor actions
- `docs/DESIGN.md` / `docs/TUI_PLAN.md` / `docs/MODULES.md` / `docs/CLI.md` / `docs/SCHEMA.md` - update only where behavior or layout changes

---

## Phase 0: Freeze Current Behavior And Map All Config Entry Points

Goal: create a complete inventory of config reads, config summaries, and TUI/doctor behavior before moving anything.

### Task 0.1: Map current config readers and writers

**Files:**
- Read: `src/config.rs`
- Read: `src/profile.rs`
- Read: `src/post/config.rs`
- Read: `src/asr/providers/apple.rs`
- Read: `src/asr/providers/doubao.rs`
- Read: `src/cli/doctor.rs`
- Read: `src/tui/settings.rs`
- Read: `src/tui/panes.rs`
- Read: `src/tui/mod.rs`
- Read: `assets/i18n/en-US.toml`
- Read: `assets/i18n/zh-CN.toml`

- [ ] Enumerate every file path, struct, and function that currently loads or summarizes configuration.
- [ ] Mark which ones are runtime loaders, which ones are diagnostics-only, and which ones are TUI-only summaries.
- [ ] Identify which pieces already have good boundaries and which are ad hoc.

Verification:
- [ ] No code changes.
- [ ] Produce a short inventory note in the working log or draft doc comments only.

### Task 0.2: Freeze existing user-visible behavior

**Files:**
- Read: `docs/DESIGN.md`
- Read: `docs/CLI.md`
- Read: `docs/MODULES.md`
- Read: `docs/TUI_PLAN.md`
- Read: `docs/SCHEMA.md`

- [ ] Record the current promise for `doctor`, reload, provider config, profile routing, and TUI settings display.
- [ ] Separate current behavior from desired behavior so later phases can change one thing at a time.

Verification:
- [ ] No code changes.

Exit criteria:

- You can point to every config entry point in the repo.
- You can state exactly which behavior later phases are allowed to change.

---

## Phase 1: Write The Config Architecture Spec

Goal: document the new `config` module tree and the single-fact-source rules before code moves.

### Task 1.1: Write the architecture spec

**Files:**
- Create: `docs/CONFIGURE_ARCHITECTURE.md`

- [ ] Describe the `config` module as the shared source of truth for parse, validation, inventory, diagnostics, and templates.
- [ ] Define the severity model: `error`, `warning`, `info`.
- [ ] Define field rules: required, defaulted, secret, enum, unknown-field handling.
- [ ] Define refresh behavior when files are edited outside the TUI.
- [ ] Define what belongs in templates and what must stay out.
- [ ] Define how `doctor` and TUI consume the same validation outputs.

Verification:
- [ ] Read the spec end-to-end for contradictions.
- [ ] Ensure the spec explicitly says templates are derived from spec, not the other way around.

### Task 1.2: Update the TUI plan to match the new module names

**Files:**
- Modify: `docs/TUI_PLAN.md`
- Modify: `docs/TODO.md`

- [ ] Rename the Configure sections to the chosen TUI labels: `Overview`, `Main`, `Profile`, `PostProcessor`, `ASR Provider`, `Theme`.
- [ ] Record that `Overview` absorbs diagnostics.
- [ ] Record that `Profile` and `PostProcessor` stay file-driven, with only limited creation flows in TUI.
- [ ] Record that `ASR Provider` is mostly view/open/edit in the first version.

Verification:
- [ ] TUI terminology matches the planned runtime architecture.

Exit criteria:

- There is one written spec that all later phases can reference.
- The spec says where external edits are handled and where validation happens.

---

## Phase 2: Convert `src/config.rs` Into A Module Tree Without Behavior Change

Goal: move the top-level config API into `src/config/mod.rs` and create room for spec/inventory/template code.

### Task 2.1: Move the top-level config module

**Files:**
- Move: `src/config.rs` -> `src/config/mod.rs`
- Create: `src/config/main.rs`

- [ ] Preserve the current public API so callers can still load the top-level config without behavior changes.
- [ ] Put the top-level schema types and path helpers into the new `main.rs` or `mod.rs` split as planned.
- [ ] Keep `default_path`, `load_from`, and `parse` stable for callers.

Verification:
- [ ] The old call sites compile unchanged or with minimal import path updates.

### Task 2.2: Add module placeholders for future config domains

**Files:**
- Create: `src/config/spec.rs`
- Create: `src/config/inventory.rs`
- Create: `src/config/template.rs`
- Create: `src/config/profile.rs`
- Create: `src/config/post/mod.rs`
- Create: `src/config/post/rule.rs`
- Create: `src/config/post/llm.rs`
- Create: `src/config/asr/mod.rs`
- Create: `src/config/asr/apple.rs`
- Create: `src/config/asr/doubao.rs`
- Create: `src/config/theme.rs`

- [ ] Wire the module tree so later phases can fill files in without another rename.
- [ ] Keep placeholder files minimal and compile-safe.

Verification:
- [ ] `cargo check` still passes after the move.

Exit criteria:

- The repository now has a `config` module root.
- No behavior change has been introduced yet.

---

## Phase 3: Introduce The Shared Spec And Validator Core

Goal: create the metadata layer that drives parse validation, TUI inventory, and template generation.

### Task 3.1: Define the spec and diagnostic types

**Files:**
- Create: `src/config/spec.rs`

- [ ] Add `Severity`, `Diagnostic`, `FieldSpec`, `ConfigSpec`, and any small helper types needed for field metadata.
- [ ] Represent required vs optional fields, defaults, secret redaction, and enum/format constraints.
- [ ] Keep the API small and explicit.

Verification:
- [ ] The types are generic enough to describe `config.toml`, profile files, ASR provider files, and post component files.

### Task 3.2: Define validation helpers that can be reused everywhere

**Files:**
- Modify: `src/config/spec.rs`
- Modify: `src/config/mod.rs`

- [ ] Add helper functions that validate parsed TOML values against a `ConfigSpec`.
- [ ] Unknown fields should emit warnings.
- [ ] Missing required fields and type mismatches should emit errors.
- [ ] Secret fields should be represented in a way the TUI can later redact.

Verification:
- [ ] The helper API can be called from doctor, inventory, and template checks.

Exit criteria:

- There is one structured place to describe config shape and validation semantics.
- Later config-domain code can reuse the same metadata.

---

## Phase 4: Move Domain Loaders Under `config`

Goal: stop treating `profile`, `post`, and ASR config as separate islands.

### Task 4.1: Move profile loading into `config::profile`

**Files:**
- Modify: `src/profile.rs`
- Create or modify: `src/config/profile.rs`

- [ ] Preserve profile route resolution behavior.
- [ ] Move the actual schema/inventory logic under `config::profile`.
- [ ] Keep the public calling surface small.

### Task 4.2: Move post component schema into `config::post`

**Files:**
- Modify: `src/post/config.rs`
- Create or modify: `src/config/post/mod.rs`
- Create or modify: `src/config/post/rule.rs`
- Create or modify: `src/config/post/llm.rs`

- [ ] Split rule vs LLM schema where it matters.
- [ ] Keep `rule` singular in the public config naming.
- [ ] Keep `llm` as the only real first-class model-backed post processor type.

### Task 4.3: Move ASR provider config loading into `config::asr`

**Files:**
- Modify: `src/asr/providers/apple.rs`
- Modify: `src/asr/providers/doubao.rs`
- Create or modify: `src/config/asr/mod.rs`
- Create or modify: `src/config/asr/apple.rs`
- Create or modify: `src/config/asr/doubao.rs`

- [ ] Keep provider runtime code where it belongs, but move schema and file-path knowledge into `config`.
- [ ] Preserve the current Apple and Doubao behavior.

Verification:
- [ ] The old provider behavior still works.
- [ ] The config module now knows where each file lives and what it should contain.

Exit criteria:

- All configuration domains are reachable from `config`.
- Runtime providers still work without behavioral drift.

---

## Phase 5: Add Inventory And Make TUI Read Structured Data

Goal: replace `tui::settings::load_rows()` with a structured inventory that understands modules, files, and status.

### Task 5.1: Build the config inventory model

**Files:**
- Create: `src/config/inventory.rs`
- Modify: `src/config/mod.rs`

- [ ] Model module groups: `Overview`, `Main`, `Profile`, `PostProcessor`, `ASR Provider`, `Theme`.
- [ ] Model file-level entries and parse status.
- [ ] Model summary text, source path, and warning/error state.

### Task 5.2: Replace the current TUI settings loader

**Files:**
- Modify: `src/tui/settings.rs`
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/panes.rs`

- [ ] Stop building settings rows by hand from ad hoc TOML parsing.
- [ ] Read the structured inventory instead.
- [ ] Keep the page read-only for now.

Verification:
- [ ] Settings/Configure still renders useful information.
- [ ] The page can survive external file edits by refreshing from inventory.

Exit criteria:

- TUI no longer depends on scattered file-specific summary logic.

---

## Phase 6: Rework Configure UI Into Module Navigation

Goal: turn the Settings page into a real Configure manager with module-centric navigation.

### Task 6.1: Replace the current page layout

**Files:**
- Modify: `src/tui/panes.rs`
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/keybindings.rs`
- Modify: `assets/i18n/en-US.toml`
- Modify: `assets/i18n/zh-CN.toml`

- [ ] Render `Overview`, `Main`, `Profile`, `PostProcessor`, `ASR Provider`, and `Theme`.
- [ ] Make `Overview` the landing module for diagnostics summary.
- [ ] Keep the footer focused on module navigation and file actions.

### Task 6.2: Add refresh behavior for external edits

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/panes.rs`

- [ ] Refresh the inventory when the page is entered.
- [ ] Refresh after editor return.
- [ ] Refresh after explicit user action.

Verification:
- [ ] The user can edit files outside the TUI and come back to updated state.

Exit criteria:

- Configure now feels like a configuration manager rather than a text table.

---

## Phase 7: Integrate Doctor Into Overview

Goal: make diagnostics visible, explicit, and manually refreshable without a separate doctor window.

### Task 7.1: Add doctor result storage to TUI state

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/panes.rs`

- [ ] Store last doctor run time, status, and captured output.
- [ ] Run doctor automatically once on first Overview entry.
- [ ] Require manual re-run after that.

### Task 7.2: Capture doctor output as a TUI action

**Files:**
- Modify: `src/cli/doctor.rs`
- Modify: `src/tui/mod.rs`

- [ ] Launch the existing `doctor` command as a subprocess when needed.
- [ ] Capture stdout/stderr and show the result in Overview.
- [ ] Keep the separate CLI command intact.

Verification:
- [ ] Doctor no longer needs its own TUI page.
- [ ] Overview shows useful health output.

Exit criteria:

- Diagnostics are integrated but not intrusive.

---

## Phase 8: Add Open/Reveal/Edit/Reload Actions

Goal: make Configure useful without building a full editor.

### Task 8.1: Add editor launcher helpers

**Files:**
- Create: `src/tui/config_actions.rs`
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/keybindings.rs`

- [ ] Resolve `$VISUAL`, then `$EDITOR`, then the macOS default editor path.
- [ ] Open the selected file in the editor.
- [ ] Open the config directory in Finder from the page.

### Task 8.2: Wire reload config and manual validation actions

**Files:**
- Modify: `src/ipc/protocol.rs`
- Modify: `src/ipc/server.rs`
- Modify: `src/tui/mod.rs`

- [ ] Keep `reload_config` available from TUI.
- [ ] Add a manual validate/refresh path that re-reads inventory and diagnostics.

Verification:
- [ ] A user can inspect config, edit it in an editor, then come back and reload/validate.

Exit criteria:

- The TUI has a sane edit loop without a built-in form editor.

---

## Phase 9: Add The Official Template Registry

Goal: make templates the canonical source for newly generated files, while keeping schema and templates in sync.

### Task 9.1: Create the template assets and manifest

**Files:**
- Create: `assets/config/manifest.toml`
- Create: `assets/config/main.toml`
- Create: `assets/config/profile/default.toml`
- Create: `assets/config/post/rule/zh_filter.toml`
- Create: `assets/config/post/llm/deepseek.toml`
- Create: `assets/config/post/llm/openai.toml`
- Create: `assets/config/post/llm/anthropic.toml`
- Create: `assets/config/post/llm/custom-openai.toml`
- Create: `assets/config/post/llm/custom-anthropic.toml`
- Create: any extra preset template files referenced by `assets/config/manifest.toml`

- [ ] Keep the templates authoritative and minimal.
- [ ] Do not copy template bodies into docs.
- [ ] Make the manifest describe what each template is for and where it can be used.

### Task 9.2: Render templates from the spec layer

**Files:**
- Modify: `src/config/template.rs`
- Modify: `src/config/spec.rs`

- [ ] Generate template text from the same field/spec metadata used for validation.
- [ ] Omit default values unless a preset intentionally overrides them.
- [ ] Redact secret placeholders appropriately.

Verification:
- [ ] The template set and the spec set cannot drift silently.

Exit criteria:

- There is one official template source of truth.

---

## Phase 10: Add The LLM Component Wizard

Goal: let the user create a new LLM post component without building a general TOML editor.

### Task 10.1: Add the creation flow and state machine

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/panes.rs`
- Modify: `src/tui/keybindings.rs`
- Modify: `assets/i18n/en-US.toml`
- Modify: `assets/i18n/zh-CN.toml`

- [ ] Support template selection.
- [ ] Support file-id entry and duplicate detection.
- [ ] Support provider-name entry and duplicate detection.
- [ ] Support `format`, `base_url`, and `model` entry.
- [ ] Leave prompt/system-prompt editing to the editor after file creation if needed.

### Task 10.2: Validate and write the new file

**Files:**
- Modify: `src/config/template.rs`
- Modify: `src/config/spec.rs`

- [ ] Validate the new component locally before writing.
- [ ] Refuse duplicate `file_id`.
- [ ] Refuse duplicate provider `name` where the schema requires uniqueness.
- [ ] Create the file, then open it in the editor.

Verification:
- [ ] Creation does not auto-attach the component to a profile.
- [ ] The user still edits profile chains manually.

Exit criteria:

- The first high-value creation flow works without a generic editor UI.

---

## Phase 11: Normalize i18n And Documentation

Goal: keep zh-CN/en-US aligned while the UI and config architecture evolve.

### Task 11.1: Align all TUI and diagnostics keys

**Files:**
- Modify: `assets/i18n/en-US.toml`
- Modify: `assets/i18n/zh-CN.toml`
- Modify: `src/i18n/mod.rs`
- Modify: `src/tui/*`

- [ ] Add keys for Configure module labels, diagnostics, validation states, template creation, editor launch, and refresh actions.
- [ ] Keep key alignment tests passing.

### Task 11.2: Update the design docs that describe config and TUI behavior

**Files:**
- Modify: `docs/DESIGN.md`
- Modify: `docs/CLI.md`
- Modify: `docs/MODULES.md`
- Modify: `docs/TUI_PLAN.md`
- Modify: `docs/SCHEMA.md`

- [ ] Update any stale references to `General`, `Settings`, or the old ad hoc summary behavior.
- [ ] Ensure the docs say templates and validation come from the same spec source.

Exit criteria:

- Code, templates, diagnostics, and docs describe the same thing.

---

## Verification Rule For Every Phase

Run these after each phase:

```bash
cargo fmt
cargo check
cargo test
git status --short --branch -uall
```

Phase-specific manual checks:

- External config edits must show up after refresh.
- Overview must show doctor output and last run state.
- Template generation must never introduce fields the schema does not know about.
- LLM creation must write only validated files.

---

## Suggested Commit Cadence

1. Phase 0-1: documentation only
2. Phase 2: module move, no behavior change
3. Phase 3-4: config spec + domain migration
4. Phase 5-6: inventory + TUI Configure skeleton
5. Phase 7-8: diagnostics + editor/reload actions
6. Phase 9-10: templates + LLM wizard
7. Phase 11: i18n and docs cleanup
