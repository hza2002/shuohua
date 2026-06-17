# Configure Architecture

This document defines the configuration architecture for the Configure TUI work
and records the implemented first pass.

## Goals

- Keep configuration files as the source of truth.
- Put all configuration parsing, validation, inventory, diagnostics, and template
  knowledge behind the `config` module.
- Make TUI Configure consume structured config data instead of parsing TOML
  directly.
- Keep official templates and validators tied to the same field/spec source.
- Allow users to edit files outside the TUI at any time.

## Non-Goals

- Do not build a generic TOML form editor.
- Do not auto-attach new LLM components to profiles.
- Do not add ASR provider creation flows in the first Configure pass.
- Do not add network validation as an automatic save step.
- Do not duplicate template bodies in docs.

## TUI Information Architecture

The Configure page uses these sections:

- `Overview`: summary of current config state and diagnostics.
- `Main`: top-level `config.toml`.
- `Profile`: `profile/*.toml`, profile routes, ASR choice, hotwords, and post
  chain summary.
- `PostProcessor`: `post/rule/*.toml` and `post/llm/*.toml`.
- `ASR Provider`: `asr/apple.toml`, `asr/doubao.toml`, and future ASR provider
  files.
- `Theme`: reserved for the theme system.

`Overview` owns diagnostics. There should not be a separate Doctor TUI page.

Doctor behavior in TUI:

- First entry into `Overview` runs diagnostics once.
- Later entries show the last result.
- Manual refresh runs diagnostics again.
- Diagnostics do not run on every page entry.

## Source Of Truth

The `config` module is the source of truth for config shape and behavior.

The source of truth is not:

- Documentation.
- TUI labels.
- Example files.
- Template preset files.
- Provider runtime code outside `config`.

Runtime provider code can own runtime behavior, network protocols, and provider
sessions. It should not own user-facing file paths, schema descriptions, or
template rules.

## Target Module Layout

```text
src/config/
  mod.rs
  main.rs
  spec.rs
  inventory.rs
  template.rs
  profile.rs
  post/
    mod.rs
    rule.rs
    llm.rs
  asr/
    mod.rs
    apple.rs
    doubao.rs
  theme.rs
```

Responsibilities:

- `mod.rs`: public API and compatibility re-exports.
- `main.rs`: top-level `config.toml` schema, parse, defaults, and path helpers.
- `spec.rs`: `ConfigSpec`, `FieldSpec`, validation helpers, diagnostics.
- `inventory.rs`: one scan of config files into structured TUI/doctor data.
- `template.rs`: template registry, rendering, validation, and LLM component creation.
- `profile.rs`: profile schema and route summaries.
- `post/rule.rs`: rule post-processor schema.
- `post/llm.rs`: LLM post-processor schema and LLM template presets.
- `asr/apple.rs`: Apple provider config schema and loader.
- `asr/doubao.rs`: Doubao provider config schema and loader.
- `theme.rs`: reserved theme schema namespace.

Existing `crate::config::default_path`, `crate::config::load_from`, and
`crate::config::parse` should remain stable through the initial module move.

## Spec Model

The spec layer describes fields and validation semantics. Serde remains the
runtime parse mechanism; the spec adds user-facing metadata and diagnostics.

A field spec needs to represent:

- Key name.
- Value kind: string, bool, integer, float, enum, array, table, free table.
- Required or optional.
- Default value, if any.
- Whether the field is secret.
- Whether the field should appear in generated templates.
- Allowed enum values, where applicable.
- Human summary label for TUI/doctor output.

The spec layer must support nested tables because current config uses sections
such as `[hotkey]`, `[voice.vad]`, `[post.llm.deepseek]`, and `[extra_body]`.

## Diagnostic Semantics

Diagnostic severities:

- `error`: config cannot be trusted for the relevant runtime path.
- `warning`: config has unknown or suspicious fields, but parse may continue.
- `info`: useful state or neutral status.

Rules:

- TOML syntax error is an `error`.
- Required field missing is an `error`.
- Type mismatch is an `error`.
- Invalid enum value is an `error`.
- Unknown field is a `warning` by default.
- Empty required secret is an `error`.
- Secret values are never printed raw.

Provider/profile overrides require two-step validation:

1. Validate the base file against its own spec.
2. Validate override keys against the target provider or post component spec.

## External Edits

The TUI must not hold writable config state.

Refresh points:

- When entering Configure.
- When returning from an editor.
- After manual refresh.
- After daemon `reload_config` response, if available.

If a wizard is about to write a new file, it must re-check the destination path
immediately before writing. Existing files are not overwritten without an
explicit confirmation flow.

No file locks are required in the first version. The TUI should treat the disk
as authoritative.

## Templates

Official templates live under `assets/config/`.

Implemented layout:

```text
assets/config/
  manifest.toml
  main.toml
  profile/
    default.toml
  post/
    rule/
      zh_filter.toml
    llm/
      deepseek.toml
      openai.toml
      anthropic.toml
      custom-openai.toml
      custom-anthropic.toml
```

Template rules:

- Templates must validate against the spec.
- Templates should omit default values unless the preset intentionally overrides
  them.
- Templates should include required fields.
- Templates should not include unsupported fields.
- Template docs should reference template file paths instead of copying bodies.

Current implementation:

- `assets/config/manifest.toml` describes all official templates.
- `assets/config/main.toml`, `assets/config/profile/default.toml`,
  `assets/config/post/rule/zh_filter.toml`, and `assets/config/post/llm/*.toml`
  are checked against `config::template` render output in tests.
- LLM component creation renders from a selected template and validates the
  generated TOML before writing.

DeepSeek is a preset, not a distinct LLM config type. It uses:

- `format = "openai"`
- DeepSeek base URL.
- DeepSeek model default.
- Optional `extra_body` values that are actually needed by that provider.

Do not expose `temperature` or similar generic-looking parameters unless the
runtime code actually consumes them and the provider behavior is meaningful for
the product.

## LLM Config Boundary

The runtime supports two interface formats:

- `openai`
- `anthropic`

The first LLM schema should cover only real fields currently consumed by code:

- `type`
- `format`
- `name`
- `base_url`
- `api_key`
- `model`
- `system_prompt`
- `prompt`
- `extra_body`

Provider-specific knobs use `extra_body` as an escape hatch. The TUI should show
that provider-specific fields exist, but should not pretend to understand every
provider-specific knob.

## Configure Edit Loop

The first useful loop is:

1. Inspect config state in TUI.
2. Open the relevant file in `$VISUAL`, `$EDITOR`, or the macOS default editor.
3. Return to TUI.
4. Refresh inventory.
5. Run diagnostics.
6. Reload daemon config when appropriate.

This intentionally avoids a full form editor while still giving users a guided
configuration workflow.

Implemented TUI actions:

- `h/l` switches Configure modules.
- `j/k` selects a visible config inventory row.
- `o` opens the selected file with `$VISUAL`, `$EDITOR`, or macOS `open`.
- `r` reveals the selected file, or the config directory from Overview.
- `v` refreshes inventory and reruns doctor.
- `R` sends daemon `reload_config`.
- In PostProcessor, `n` starts the LLM component wizard.

The LLM wizard writes only `post/llm/<file_id>.toml`. It checks duplicate file
ids and duplicate provider `name` values immediately before writing. It does
not edit profile chains.

## Phase Boundaries

Implementation should follow `docs/superpowers/plans/2026-06-17-configure-refactor-roadmap.md`.

Implemented first-pass phases:

1. Move `src/config.rs` into a `src/config/` module tree with no behavior change.
2. Add spec and diagnostics types.
3. Move config domain loaders under `config`.
4. Replace TUI ad hoc settings rows with structured inventory.
5. Rework TUI Configure layout.

Remaining work is to deepen structured diagnostics so TUI and `doctor` consume
the same diagnostic objects instead of showing doctor stdout in Overview.
