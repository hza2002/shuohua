# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近 commit

当前待提交：Phase 1 platform capability model。

## 当前 phase

Phase 1: Platform Capability Model 已完成自动验证；macOS 手动 checklist 仍未由 agent 执行。

## 已完成事项

- Phase 0:
- 新增 `docs/cross-platform/macos-baseline.md`，记录自动验证基线、macOS 手动验证 checklist、
  当前允许的 macOS-only 边界和后续阶段要处理的遗留边界。
- 在 `docs/cross-platform/README.md` 增加 macOS baseline 阅读路由。
- 扩展 `tests/platform_layout.rs`：
  - `src/platform/mod.rs` 必须 cfg-gate macOS backend。
  - 共享 platform facade 不直接 import Apple SDK token。
  - 非 allowlist 业务层不能直接引用 `platform::macos`。
- Phase 1:
  - 更新 `docs/cross-platform/platform-capabilities.md`，明确 capability/status 字段、
    macOS 初始映射、非 macOS structured unsupported 和本阶段非目标。
  - 新增 `src/platform/capability.rs`，提供 `CapabilityId`、`CapabilityStatusKind`、
    `PlatformKind`、`CapabilityStatus` 和 `current_platform_capabilities()`。
  - macOS 静态快照映射现有 backend；非 macOS 快照返回 `unsupported` +
    `backend_not_implemented`。
  - `shuo doctor` 只读打印 capability summary，不改变错误/警告计数或控制流。
  - `tests/platform_layout.rs` 继续保护 capability 模块作为 shared platform 边界。

## 验证结果

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：通过。
- `cargo test --test platform_layout`：通过，5 个测试执行。
- `cargo test platform::capability`：通过，6 个测试执行。
- `cargo test cli::doctor::tests::platform_capability_summary_counts_status_kinds`：通过。
- `cargo test --test doc_consistency`：通过。
- macOS 权限、录音、overlay、clipboard/paste、TUI、service lifecycle、history 手动体验：未执行，需用户在真实 macOS 会话按 `macos-baseline.md` checklist 验证。

## 已知风险

- `src/ipc/{client,server}.rs` 和 `src/daemon/{lock,fallback}.rs` 仍依赖 UDS / Unix primitives；
  这是 Phase 3/4 的目标，不应在 Phase 2 提前抽。
- `src/cli/doctor.rs` 仍有 launchd-centric 诊断输出；Phase 4 后应通过 capability/status
  和 service manager 模型收敛。
- `src/post/app_context.rs` 当前作为 post 平台入口直接转发到 macOS app context；Phase 5
  desktop capability boundary 可统一 facade。
- `current_platform_capabilities()` 是 Phase 1 静态快照，不执行权限 probe；后续消费方不要把
  静态 `desktop.permissions=available` 误解为当前已授权。

## 下一步

进入 Phase 2: Config And Theme Cross-Platform Rules。范围保持窄：明确通用字段、平台段、
metadata、advanced 字段规则；不马上改所有 theme，不引入 GUI 配置编辑器。

建议下一 session prompt：

```text
继续 /Users/ghot/repo/shuohua 跨平台改造，当前在 feat/cross-platform-design。
先读 AGENTS.md、TODO、docs/cross-platform/README.md、overview.md、
development-plan.md、config-theme.md、platform-capabilities.md、macos-baseline.md、
handoff.md。
从 Phase 2 Config And Theme Cross-Platform Rules 开始：先更新 config-theme.md，
再写最小测试，最后实现最小 schema/template 调整。不要改所有 theme，不引入 GUI
配置编辑器，不抽 IPC/hotkey。提交前跑 cargo fmt --check &&
cargo clippy --all-targets -- -D warnings && cargo test。
```
