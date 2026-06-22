# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近 commit

HEAD: `test: guard config theme platform rules`

## 当前 phase

Phase 2: Config And Theme Cross-Platform Rules 已完成并提交。下一步进入 Phase 3。

## 已完成事项

- Phase 0:
  - 新增 `docs/cross-platform/macos-baseline.md`，记录自动验证基线、macOS 手动验证 checklist、
    当前允许的 macOS-only 边界和后续阶段要处理的遗留边界。
  - 在 `docs/cross-platform/README.md` 增加 macOS baseline 阅读路由。
  - 扩展 `tests/platform_layout.rs`，保护 shared platform facade 和 macOS-only import 边界。
- Phase 1:
  - 新增 `src/platform/capability.rs`，提供共享 capability/status 类型和静态快照。
  - macOS 快照映射现有 backend；非 macOS 快照返回 `unsupported` +
    `backend_not_implemented`。
  - `shuo doctor` 只读打印 capability summary，不改变错误/警告计数或控制流。
- Phase 2 当前改动:
  - 更新 `docs/cross-platform/config-theme.md`，明确 `[dev]` 不属于同步契约、starter config 不默认输出实验字段、
    future 平台 theme 段必须受 schema 控制。
  - 更新 `docs/cross-platform/overview.md` 阶段状态。
  - `src/config/schema.rs` 增加受控的 `overlay.windows.material` 和 `overlay.linux.material`
    schema 字段，继续拒绝平台段内 unknown typo。
  - `src/config/template/registry.rs` 从 starter config 移除默认 `[dev]` 输出；现有
    `dev.vad_trace` 配置仍可 parse。
  - 补充新增 theme 字段的 en-US / zh-CN description keys。

## 验证结果

- 已跑：`cargo test config::schema::tests`，通过 7 个 schema 单测。
- 已跑：`cargo fmt --check`，通过。
- 已跑：`cargo clippy --all-targets -- -D warnings`，通过。
- 已跑：`cargo test`，通过：629 个 unit tests、5 个 `apple_helper_build` tests、
  1 个 `cli_runtime_boundary` test、2 个 `doc_consistency` tests、5 个 `platform_layout` tests、
  6 个 `theme_registry_build` tests。
- macOS 权限、录音、overlay、clipboard/paste、TUI、service lifecycle、history 手动体验：未执行，
  需用户在真实 macOS 会话按 `macos-baseline.md` checklist 验证。

## 已知风险

- `src/ipc/{client,server}.rs` 和 `src/daemon/{lock,fallback}.rs` 仍依赖 UDS / Unix primitives；
  这是 Phase 3/4 的目标，不应在 Phase 2 提前抽。
- `src/cli/doctor.rs` 仍有 launchd-centric 诊断输出；Phase 4 后应通过 capability/status
  和 service manager 模型收敛。
- `src/post/app_context.rs` 当前作为 post 平台入口直接转发到 macOS app context；Phase 5
  desktop capability boundary 可统一 facade。
- `current_platform_capabilities()` 是 Phase 1 静态快照，不执行权限 probe；后续消费方不要把
  静态 `desktop.permissions=available` 误解为当前已授权。
- Phase 2 只让 schema 接受 future 平台 theme 偏好；macOS runtime 仍不消费
  `overlay.windows` / `overlay.linux`。

## 下一步

进入 Phase 3: IPC Transport Boundary。

建议下一 session prompt：

```text
继续 /Users/ghot/repo/shuohua 跨平台改造，当前在 feat/cross-platform-design。
先读 AGENTS.md、TODO、docs/cross-platform/README.md、overview.md、
development-plan.md、config-theme.md、platform-capabilities.md、macos-baseline.md、
handoff.md。
从 Phase 3 IPC Transport Boundary 开始：先更新 ipc-service.md，再写最小测试，最后实现
transport facade。不要改变 JSON-line command/event shape，不要改 history schema，
不要提前抽 service manager。
```
