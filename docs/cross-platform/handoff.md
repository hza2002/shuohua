# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近 commit

当前 `HEAD`：`test: guard platform boundary imports`

## 当前 phase

Phase 0: Baseline Audit 已完成自动基线产出；macOS 手动 checklist 已记录但未由 agent 执行。

## 已完成事项

- 新增 `docs/cross-platform/macos-baseline.md`，记录自动验证基线、macOS 手动验证 checklist、
  当前允许的 macOS-only 边界和后续阶段要处理的遗留边界。
- 在 `docs/cross-platform/README.md` 增加 macOS baseline 阅读路由。
- 扩展 `tests/platform_layout.rs`：
  - `src/platform/mod.rs` 必须 cfg-gate macOS backend。
  - 共享 platform facade 不直接 import Apple SDK token。
  - 非 allowlist 业务层不能直接引用 `platform::macos`。

## 验证结果

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：通过。
- `cargo test --test platform_layout`：通过，5 个测试执行。
- macOS 权限、录音、overlay、clipboard/paste、TUI、service lifecycle、history 手动体验：未执行，需用户在真实 macOS 会话按 `macos-baseline.md` checklist 验证。

## 已知风险

- `src/ipc/{client,server}.rs` 和 `src/daemon/{lock,fallback}.rs` 仍依赖 UDS / Unix primitives；
  这是 Phase 3/4 的目标，不应在 Phase 1 提前抽。
- `src/cli/doctor.rs` 仍有 launchd-centric 诊断输出；Phase 1/4 后应通过 capability/status
  和 service manager 模型收敛。
- `src/post/app_context.rs` 当前作为 post 平台入口直接转发到 macOS app context；Phase 5
  desktop capability boundary 可统一 facade。

## 下一步

进入 Phase 1: Platform Capability Model。范围保持窄：新增共享 capability/status 类型，
映射 macOS current status，非 macOS 返回 structured unsupported；不要抽 hotkey、不要改 IPC、
不要实现 Linux/Windows backend。

建议下一 session prompt：

```text
继续 /Users/ghot/repo/shuohua 跨平台改造，当前在 feat/cross-platform-design。
先读 AGENTS.md、TODO、docs/cross-platform/README.md、overview.md、
development-plan.md、platform-capabilities.md、macos-baseline.md、handoff.md。
从 Phase 1 Platform Capability Model 开始：先更新 platform-capabilities.md，
再写最小测试，最后实现共享 capability/status 类型。不要抽 hotkey、不改 IPC、
不实现 Linux/Windows backend。提交前跑 cargo fmt --check &&
cargo clippy --all-targets -- -D warnings && cargo test。
```
