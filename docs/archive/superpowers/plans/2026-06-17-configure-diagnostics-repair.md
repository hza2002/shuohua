# Configure Diagnostics Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the Configure refactor review findings by making config validation complete, nested-path aware, explicit in TUI, and safer for editor/Finder actions.

**Architecture:** Keep config files as source of truth. Add one structured local diagnostics scan in `config` that both `shuo doctor` and TUI Configure consume; keep network/provider connectivity as an explicit doctor mode. TUI Configure refreshes inventory on entry but only runs diagnostics when the user asks.

**Tech Stack:** Rust, anyhow, clap, serde/toml, tokio subprocess boundaries, ratatui/crossterm, existing unit tests in `src/main.rs`.

---

## File Map

- Modify `src/config/spec.rs`: support dotted/nested TOML paths and recursive unknown-field validation.
- Create `src/config/diagnostics.rs`: scan every config file and produce structured diagnostics.
- Modify `src/config/mod.rs`: export `diagnostics`.
- Modify `src/config/inventory.rs`: build Overview summary after scanning and surface diagnostic status consistently.
- Modify `src/cli/doctor.rs`: consume `config::diagnostics`; split local checks from explicit network/full checks.
- Modify `src/cli/mod.rs`: add doctor flags if needed.
- Modify `src/tui/mod.rs`: remove automatic doctor execution; keep manual `v`.
- Modify `src/tui/panes.rs`: update Overview text for manual diagnostics.
- Modify `src/tui/config_actions.rs`: robust editor fallback and missing-path reveal behavior.
- Modify `assets/i18n/en-US.toml` and `assets/i18n/zh-CN.toml`: align text with manual validation.
- Modify `docs/CLI.md`, `docs/CONFIGURE_ARCHITECTURE.md`, `docs/TUI_PLAN.md`, `docs/MODULES.md`: reflect local vs network diagnostics and manual TUI trigger.

---

## Task 1: Fix Nested TOML Path Validation

**Files:**
- Modify: `src/config/spec.rs`

- [ ] **Step 1: Add failing tests for dotted paths**

Add these tests under `config::spec::tests`:

```rust
#[test]
fn validate_reads_nested_dotted_paths() {
    let spec = ConfigSpec::new("main")
        .field(FieldSpec::string("hotkey.trigger").required())
        .field(
            FieldSpec::string("voice.vad.backend")
                .default("off")
                .allowed_values(["off", "silero"]),
        );
    let value: toml::Value = toml::toml! {
        [hotkey]
        trigger = "f16"

        [voice.vad]
        backend = "silero"
    }
    .into();

    let diagnostics = validate_value(&spec, &value);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn validate_reports_nested_unknown_fields() {
    let spec = ConfigSpec::new("main")
        .field(FieldSpec::string("hotkey.trigger").required())
        .field(FieldSpec::table("voice").optional())
        .field(FieldSpec::string("voice.vad.backend").optional());
    let value: toml::Value = toml::toml! {
        [hotkey]
        trigger = "f16"

        [voice.vad]
        backend = "off"
        typo = true
    }
    .into();

    let diagnostics = validate_value(&spec, &value);

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == Severity::Warning
            && diagnostic.path == "voice.vad.typo"
            && diagnostic.message.contains("unknown")
    }));
}
```

- [ ] **Step 2: Run failing tests**

Run:

```bash
cargo test config::spec::tests::validate_reads_nested_dotted_paths config::spec::tests::validate_reports_nested_unknown_fields
```

Expected: at least one test fails because `validate_value()` only reads top-level keys today.

- [ ] **Step 3: Implement dotted path lookup**

In `src/config/spec.rs`, replace direct `table.get(&field.name)` lookup with helpers:

```rust
fn value_at_path<'a>(value: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.as_table()?.get(part)?;
    }
    Some(current)
}

fn parent_path(path: &str) -> Option<&str> {
    path.rsplit_once('.').map(|(parent, _)| parent)
}
```

Use `value_at_path(value, field.name())` for required/type validation.

- [ ] **Step 4: Implement recursive unknown-field validation**

Add a helper that walks TOML tables and suppresses unknown warnings when any ancestor field is marked `free_table`:

```rust
fn collect_unknown_fields(
    spec: &ConfigSpec,
    value: &toml::Value,
    prefix: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if spec.field_for_path(&path).is_some() {
            collect_unknown_fields(spec, child, &path, diagnostics);
            continue;
        }
        if is_under_free_table(spec, &path) {
            continue;
        }
        if child.is_table() && has_descendant_field(spec, &path) {
            collect_unknown_fields(spec, child, &path, diagnostics);
            continue;
        }
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            path,
            message: "unknown field".to_string(),
        });
    }
}
```

Implement `is_under_free_table()` by checking ancestors with `parent_path()`, and `has_descendant_field()` by checking whether any spec field starts with `"{path}."`.

- [ ] **Step 5: Run spec tests**

Run:

```bash
cargo test config::spec::tests
```

Expected: all spec tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/config/spec.rs
git commit -m "Fix nested config spec validation"
```

---

## Task 2: Add Structured Local Config Diagnostics

**Files:**
- Create: `src/config/diagnostics.rs`
- Modify: `src/config/mod.rs`
- Modify: `src/config/template.rs` tests if needed

- [ ] **Step 1: Add diagnostics module skeleton**

Create `src/config/diagnostics.rs`:

```rust
use std::path::{Path, PathBuf};

use crate::config::spec::{Diagnostic, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticScope {
    Main,
    Profile,
    AsrProvider,
    PostProcessor,
    Template,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiagnostic {
    pub scope: DiagnosticScope,
    pub source: PathBuf,
    pub severity: Severity,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiagnosticReport {
    pub root: PathBuf,
    pub diagnostics: Vec<ConfigDiagnostic>,
    pub files_checked: usize,
}

impl ConfigDiagnosticReport {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

pub fn run_local() -> ConfigDiagnosticReport {
    run_local_from_config_home(&config_home())
}

pub fn run_local_from_config_home(config_home: &Path) -> ConfigDiagnosticReport {
    let root = config_home.join("shuohua");
    let mut report = ConfigDiagnosticReport {
        root: root.clone(),
        diagnostics: Vec::new(),
        files_checked: 0,
    };
    scan_main(&mut report, &root);
    scan_profiles(&mut report, &root);
    scan_asr(&mut report, &root);
    scan_post(&mut report, &root);
    scan_templates(&mut report);
    report
}

fn config_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
}
```

Export it in `src/config/mod.rs`:

```rust
pub mod diagnostics;
```

- [ ] **Step 2: Add failing tests for full local scan**

Add tests in `diagnostics.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn temp_config_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shuohua-diagnostics-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn local_diagnostics_scans_unreferenced_profile_asr_and_post_files() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(root.join("profile/broken.toml"), "[asr\n").unwrap();
        fs::write(root.join("asr/broken.toml"), "idle_pause = true\nunknown = 1\n").unwrap();
        fs::write(root.join("post/llm/broken.toml"), "type = \"llm\"\napi_key = \"\"\n").unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.files_checked >= 4);
        assert!(report.diagnostics.iter().any(|d| d.source.ends_with("profile/broken.toml")));
        assert!(report.diagnostics.iter().any(|d| d.source.ends_with("asr/broken.toml")));
        assert!(report.diagnostics.iter().any(|d| d.source.ends_with("post/llm/broken.toml")));
        let _ = fs::remove_dir_all(home);
    }
}
```

- [ ] **Step 3: Implement file scanning helpers**

Implement:

```rust
fn scan_main(report: &mut ConfigDiagnosticReport, root: &Path) { /* read config.toml, parse, validate */ }
fn scan_profiles(report: &mut ConfigDiagnosticReport, root: &Path) { /* profile/*.toml */ }
fn scan_asr(report: &mut ConfigDiagnosticReport, root: &Path) { /* asr/*.toml */ }
fn scan_post(report: &mut ConfigDiagnosticReport, root: &Path) { /* post/rule/*.toml and post/llm/*.toml */ }
fn scan_templates(report: &mut ConfigDiagnosticReport) { /* assets/config registry render + spec validate */ }
fn toml_files(dir: &Path) -> Vec<PathBuf> { /* sorted .toml files */ }
fn push_parse_error(report: &mut ConfigDiagnosticReport, scope: DiagnosticScope, source: PathBuf, error: anyhow::Error) { /* severity error */ }
fn push_spec_diagnostics(report: &mut ConfigDiagnosticReport, scope: DiagnosticScope, source: &Path, diagnostics: Vec<Diagnostic>) { /* map Diagnostic */ }
```

Keep this local-only. Do not perform network calls here.

- [ ] **Step 4: Add reference checks**

Add checks that profile post chain references exist:

```rust
fn validate_profile_references(report: &mut ConfigDiagnosticReport, root: &Path, source: &Path, profile: &crate::config::profile::Profile) {
    for item in &profile.post.chain {
        let Some((kind, name)) = item.split_once(':') else {
            push_error(report, DiagnosticScope::Profile, source, "post.chain", format!("post chain item {item:?} must be kind:name"));
            continue;
        };
        let path = match kind {
            "rule" => root.join("post/rule").join(format!("{name}.toml")),
            "llm" => root.join("post/llm").join(format!("{name}.toml")),
            other => {
                push_error(report, DiagnosticScope::Profile, source, "post.chain", format!("unknown post component kind {other:?}"));
                continue;
            }
        };
        if !path.exists() {
            push_error(report, DiagnosticScope::Profile, source, "post.chain", format!("missing post component {}", path.display()));
        }
    }
}
```

Also check profile ASR provider has a known provider name (`apple` or `doubao`) and that `asr/<provider>.toml` exists when that provider requires a file.

- [ ] **Step 5: Run diagnostics tests**

Run:

```bash
cargo test config::diagnostics::tests
```

Expected: all diagnostics tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/config/mod.rs src/config/diagnostics.rs src/config/template.rs
git commit -m "Add structured config diagnostics"
```

---

## Task 3: Split Doctor Into Local and Explicit Network Checks

**Files:**
- Modify: `src/cli/doctor.rs`
- Modify: `docs/CLI.md`

- [ ] **Step 1: Add doctor flags**

Update `DoctorArgs`:

```rust
#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Run provider network/auth checks without sending audio.
    #[arg(long)]
    pub network: bool,

    /// Run full checks that may touch real provider paths. Still do not send PCM unless implemented explicitly.
    #[arg(long)]
    pub full: bool,
}
```

- [ ] **Step 2: Print structured local diagnostics**

In `check_config()`, call `crate::config::diagnostics::run_local()` and print:

```rust
fn check_config() -> bool {
    let report = crate::config::diagnostics::run_local();
    println!(
        "config.local: checked {} files under {}",
        report.files_checked,
        report.root.display()
    );
    for diagnostic in &report.diagnostics {
        println!(
            "config.{:?}: {:?} {} {}: {}",
            diagnostic.scope,
            diagnostic.severity,
            diagnostic.source.display(),
            diagnostic.path,
            diagnostic.message
        );
    }
    if report.has_errors() {
        println!("config.local: ERROR");
        false
    } else {
        println!("config.local: OK");
        true
    }
}
```

Keep effective config output after this, but only if main config parses.

- [ ] **Step 3: Gate network checks**

Change `run()` flow:

```rust
let config_ok = check_config();
check_hotkey();
check_microphone_input();
check_uds();
check_launchd();
check_permissions();
if args.network || args.full {
    check_asr_provider();
    check_llm_processors();
} else {
    println!("network: skipped (run `shuo doctor --network` to test ASR/LLM connectivity)");
}
```

Add `check_llm_processors()` as a placeholder local network hook that detects configured LLM processors and prints that LLM network checks are skipped unless implemented. Do not send user text.

- [ ] **Step 4: Add tests for local report formatting where practical**

If `doctor.rs` remains hard to unit test due to stdout, keep tests in `config::diagnostics` and add one narrow pure helper test if a formatter helper is introduced.

- [ ] **Step 5: Run doctor manually**

Run:

```bash
cargo run -- doctor
cargo run -- doctor --network
```

Expected:
- plain doctor checks local config and prints network skipped.
- `--network` attempts explicit provider/LLM connectivity checks or prints scoped skipped messages.

- [ ] **Step 6: Commit**

```bash
git add src/cli/doctor.rs docs/CLI.md
git commit -m "Split doctor local and network checks"
```

---

## Task 4: Fix Inventory Overview Summary

**Files:**
- Modify: `src/config/inventory.rs`

- [ ] **Step 1: Add failing test**

Add:

```rust
#[test]
fn overview_summary_counts_after_scan() {
    let home = temp_config_home();
    let root = home.join("shuohua");
    fs::create_dir_all(root.join("profile")).unwrap();
    fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
    fs::write(root.join("profile/default.toml"), "name = \"default\"\n[asr]\nprovider = \"apple\"\n").unwrap();

    let inventory = load_from_config_home(&home);
    let overview = inventory
        .entries()
        .find(|entry| entry.module == InventoryModule::Overview && entry.key == "summary")
        .unwrap();

    assert!(!overview.summary.starts_with("0 "), "{overview:?}");
    let _ = fs::remove_dir_all(home);
}
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cargo test config::inventory::tests::overview_summary_counts_after_scan
```

Expected: fails with current `0 config files scanned` behavior.

- [ ] **Step 3: Move overview generation after scanning**

Change order in `load_from_config_home()`:

```rust
push_main(&mut inventory, &root);
push_profiles(&mut inventory, &root);
push_post(&mut inventory, &root);
push_asr(&mut inventory, &root);
push_theme_placeholder(&mut inventory, &root);
push_overview(&mut inventory);
```

Update `push_overview()` to count non-overview entries and preferably unique source files:

```rust
let total_files = inventory
    .entries()
    .filter(|entry| entry.module != InventoryModule::Overview)
    .map(|entry| entry.source.clone())
    .collect::<std::collections::BTreeSet<_>>()
    .len();
```

- [ ] **Step 4: Run inventory tests**

Run:

```bash
cargo test config::inventory::tests
```

Expected: all inventory tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config/inventory.rs
git commit -m "Fix Configure inventory overview count"
```

---

## Task 5: Make TUI Doctor Manual Only

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/panes.rs`
- Modify: `assets/i18n/en-US.toml`
- Modify: `assets/i18n/zh-CN.toml`

- [ ] **Step 1: Remove automatic doctor trigger**

In `on_page_changed()`, keep:

```rust
if app.page == Page::Settings {
    app.refresh_configure();
}
```

Remove `maybe_run_doctor(app)` calls from page/module changes. `v` remains the only trigger:

```rust
Action::ValidateConfig => {
    if app.page == Page::Settings {
        app.refresh_configure();
        app.doctor = run_doctor();
        app.status = crate::t!("tui.configure.validated");
    }
}
```

- [ ] **Step 2: Simplify or remove `maybe_run_doctor()`**

Delete `maybe_run_doctor()` if unused.

- [ ] **Step 3: Update Overview wording**

Change i18n text:

```toml
doctor_not_run = "doctor has not run in this TUI session; press v to validate"
```

Chinese:

```toml
doctor_not_run = "本次 TUI 会话尚未运行 doctor；按 v 校验"
```

- [ ] **Step 4: Run TUI tests**

Run:

```bash
cargo test tui::tests tui::panes::tests tui::keybindings::tests i18n::tests::zh_cn_and_en_us_keys_match
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/tui/mod.rs src/tui/panes.rs assets/i18n/en-US.toml assets/i18n/zh-CN.toml
git commit -m "Make Configure diagnostics manual"
```

---

## Task 6: Add Editor Fallback and Safer Finder Reveal

**Files:**
- Modify: `src/tui/config_actions.rs`

- [ ] **Step 1: Add tests for editor fallback**

Add:

```rust
#[test]
fn editor_launch_splits_program_and_args() {
    let path = Path::new("/tmp/config.toml");

    assert_eq!(
        editor_launch_for(path, Some("nvim -f"), None),
        EditorLaunch::Command {
            program: "nvim".to_string(),
            args: vec!["-f".to_string(), "/tmp/config.toml".to_string()],
        }
    );
}
```

- [ ] **Step 2: Implement simple whitespace split**

Update `editor_launch_for()`:

```rust
if let Some(command) = non_empty(visual).or_else(|| non_empty(editor)) {
    let mut parts = command.split_whitespace();
    if let Some(program) = parts.next() {
        let mut args = parts.map(str::to_string).collect::<Vec<_>>();
        args.push(path);
        return EditorLaunch::Command {
            program: program.to_string(),
            args,
        };
    }
}
```

This intentionally supports common `nvim -f` / `code --wait` forms without adding shell parsing.

- [ ] **Step 3: Add runtime fallback to macOS open**

In `open_in_editor()`, if `Command::new(program).args(args).spawn()` fails, run `open <path>`:

```rust
fn mac_open(path: &str) -> Result<()> {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .with_context(|| format!("open {path}"))?;
    Ok(())
}
```

Use it after failed custom editor launch.

- [ ] **Step 4: Add reveal target resolver tests**

Create a pure helper:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum RevealLaunch {
    RevealFile(PathBuf),
    OpenDir(PathBuf),
}
```

Add tests:

```rust
#[test]
fn reveal_opens_parent_for_missing_file() {
    let dir = std::env::temp_dir().join(format!("shuohua-reveal-test-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("config.toml");

    assert_eq!(reveal_launch_for(&missing), Some(RevealLaunch::OpenDir(dir.clone())));
    let _ = std::fs::remove_dir_all(dir);
}
```

- [ ] **Step 5: Implement missing-path reveal behavior**

Implement:

```rust
fn reveal_launch_for(path: &Path) -> Option<RevealLaunch> {
    if path.is_file() {
        return Some(RevealLaunch::RevealFile(path.to_path_buf()));
    }
    if path.is_dir() {
        return Some(RevealLaunch::OpenDir(path.to_path_buf()));
    }
    path.parent()
        .filter(|parent| parent.exists())
        .map(|parent| RevealLaunch::OpenDir(parent.to_path_buf()))
}
```

Use:

```rust
match reveal_launch_for(path) {
    Some(RevealLaunch::RevealFile(path)) => Command::new("open").arg("-R").arg(path).spawn()?,
    Some(RevealLaunch::OpenDir(path)) => Command::new("open").arg(path).spawn()?,
    None => anyhow::bail!("config path and parent do not exist: {}", path.display()),
}
```

- [ ] **Step 6: Run config action tests**

Run:

```bash
cargo test tui::config_actions::tests
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/tui/config_actions.rs
git commit -m "Harden Configure editor and reveal actions"
```

---

## Task 7: Align Docs With New Behavior

**Files:**
- Modify: `docs/CLI.md`
- Modify: `docs/CONFIGURE_ARCHITECTURE.md`
- Modify: `docs/TUI_PLAN.md`
- Modify: `docs/MODULES.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Update doctor docs**

Document:

- `shuo doctor` = local deterministic checks.
- `shuo doctor --network` = explicit ASR/LLM connectivity checks.

- [ ] **Step 2: Update Configure docs**

Replace auto-doctor statements with manual trigger:

```text
Configure refreshes inventory on entry. It does not run doctor automatically.
Users press `v` to run local diagnostics. Network checks remain explicit through
`shuo doctor --network` and are not run from TUI by default.
```

- [ ] **Step 3: Update module responsibility docs**

Document `config::diagnostics`:

```text
src/config/diagnostics.rs - local full-tree config diagnostics shared by doctor and TUI.
```

- [ ] **Step 4: Run doc grep**

Run:

```bash
rg -n "First entry into `Overview` runs|auto.*doctor|runs diagnostics once|M5 scope" docs assets src
```

Expected: no stale auto-doctor claims remain.

- [ ] **Step 5: Commit**

```bash
git add docs/CLI.md docs/CONFIGURE_ARCHITECTURE.md docs/TUI_PLAN.md docs/MODULES.md CHANGELOG.md
git commit -m "Align Configure diagnostics docs"
```

---

## Final Verification

- [ ] Run formatting:

```bash
cargo fmt
```

- [ ] Run full checks:

```bash
cargo check
cargo test
```

- [ ] Run doctor manually:

```bash
cargo run -- doctor
cargo run -- doctor --network
```

- [ ] Inspect git state:

```bash
git status --short --branch -uall
```

- [ ] Manual macOS checks for user:

```text
1. Open TUI Configure.
2. Confirm it does not run doctor automatically.
3. Press v and confirm diagnostics output appears.
4. Press o on a config file and confirm editor/open fallback works.
5. Press r on an existing file, missing file, and config directory.
6. Press R and confirm daemon reload still works.
```

---

## Self-Review

- Covers local vs network doctor split: Task 3.
- Covers nested TOML paths: Task 1.
- Covers Overview count: Task 4.
- Covers manual TUI trigger: Task 5.
- Covers editor fallback: Task 6.
- Covers Finder reveal missing paths: Task 6.
- Covers docs: Task 7.
- No runtime code should be changed outside the listed files.
