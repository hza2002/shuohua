# Overlay UI Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refine the macOS overlay into a tighter two-row HUD with clearer status hierarchy, better spacing, centralized style constants, and visible segment/partial distinction.

**Architecture:** Keep the existing single `NSPanel` and `OverlayCmd` model. Concentrate visual constants in `src/overlay/view.rs`, add pure helper functions for layout/text planning, and use attributed text for the body while avoiding a broader view split.

**Tech Stack:** Rust, objc2 AppKit bindings, existing unit tests.

---

### Task 1: Layout And Text Planning Tests

**Files:**
- Modify: `src/overlay/view.rs`

- [ ] Add tests for the compact first-row frame plan: state remains prominent, duration/words/app are left clustered, and meta receives wider remaining space.
- [ ] Add tests for recording text planning: segments and partial remain separate so the view can render them with different colors.
- [ ] Run the targeted overlay tests and confirm the new tests fail before implementation.

### Task 2: Centralize Style Constants

**Files:**
- Modify: `src/overlay/mod.rs`
- Modify: `src/overlay/view.rs`

- [ ] Replace scattered overlay colors with a local Gruvbox palette and semantic overlay colors.
- [ ] Replace top-level frame magic numbers with grouped layout/typography constants in `view.rs`.
- [ ] Keep the public overlay command/model API unchanged.

### Task 3: First Row And Body Spacing

**Files:**
- Modify: `src/overlay/view.rs`

- [ ] Rework first-row frames into: icon/state, duration, words, app, meta.
- [ ] Make state text larger/bolder than stats/meta.
- [ ] Improve body frame padding and line height so the text area no longer looks like it has an extra unused row.
- [ ] Keep notice overriding meta; chain is displayed more quietly when no notice is active.

### Task 4: Attributed Body Text And Light Motion

**Files:**
- Modify: `src/overlay/view.rs`

- [ ] Render segments as dim text and partial as primary text using `NSAttributedString`.
- [ ] Keep final/error behavior intact, with error in red.
- [ ] Add low-cost show/hide alpha behavior and state icon breathing for non-recording active states.
- [ ] Avoid per-partial whole-text fade that makes live recognition feel noisy.

### Task 5: Verification And Commit

**Files:**
- Modify: `src/overlay/mod.rs`
- Modify: `src/overlay/view.rs`
- Add: `docs/superpowers/plans/2026-06-16-overlay-ui-polish.md`

- [ ] Run `cargo fmt`.
- [ ] Run `cargo check`.
- [ ] Run `cargo test`.
- [ ] Inspect `git status --short --branch -uall`.
- [ ] Commit only this stage.
