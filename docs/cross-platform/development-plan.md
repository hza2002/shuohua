# Cross-Platform Development Plan

## 原则

跨平台改造必须小步推进。每个阶段只改变一个边界，先保护 macOS 行为，再加入新平台能力。

每个阶段的默认流程：

1. 更新对应设计文档，写清当前判断、风险和验收。
2. 写最小测试或检查，先证明现状/目标边界。
3. 做最小实现，不顺手重构无关模块。
4. 跑受影响测试。
5. 若触及共享边界，跑 `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`。
6. 手动验证项不能由自动测试替代时，在提交说明里明确未手动验证的范围。

macOS 当前可用版本是回归基线。任何阶段如果破坏 macOS hotkey、录音、ASR、post、clipboard、
paste、overlay、TUI、history 或 service lifecycle，应先回滚或修复该阶段，再继续。

## Phase 0: Baseline Audit

目标：建立重构前基线，避免后续不知道是否回退。

范围：

- 记录当前 macOS 关键路径。
- 补齐缺失的 platform-boundary 测试。
- 审计配置字段、平台 facade、macOS-only import。

验收：

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- 用户手动验证 macOS 录音、停止、取消、overlay、上屏、TUI/service。

## Phase 1: Platform Capability Model

目标：先统一表达能力状态，不改变现有行为。

范围：

- 新增共享 capability/status 类型。
- doctor/TUI 可以读取但不强依赖。
- macOS 现有能力先映射为 current status。
- 非 macOS 先返回 structured unsupported。

不做：

- 不抽 hotkey backend。
- 不改 IPC protocol。
- 不新增 Linux/Windows 实现。

验收：

- macOS 行为不变。
- unsupported/partial/degraded/unavailable 能被测试覆盖。
- doctor 输出不退化。

## Phase 2: Config And Theme Cross-Platform Rules

目标：让配置和 theme 具备平台扩展空间，但不破坏现有配置。

范围：

- 明确通用字段、平台段、metadata、advanced 字段规则。
- 降低实验字段在 starter config 中的存在感。
- 为 overlay material preference 增加 schema 设计前评审。

不做：

- 不马上改所有 theme。
- 不引入 GUI 配置编辑器。

验收：

- 现有用户配置继续 parse。
- 官方模板字段都有 schema 和使用点或 metadata 说明。
- unknown typo 仍被诊断。

## Phase 3: IPC Transport Boundary

目标：协议不变，transport 可替换。

范围：

- 抽 client/server transport facade。
- macOS 继续 Unix domain socket。
- Linux 可复用 Unix domain socket。
- Windows 只设计 Named Pipe adapter，不急于完整实现。

不做：

- 不改 JSON-line command/event shape。
- 不改 history schema。

验收：

- 现有 IPC tests 通过。
- TUI 对 transport 细节无感。
- stale socket 清理仍只在 daemon lock owner 路径。

## Phase 4: Single Instance, Process Probe, Service Manager

目标：把 daemon 生命周期从 macOS launchd 细节中拆出来。

范围：

- 抽单实例 lock facade。
- 抽 process probe。
- 抽 service manager facade。
- macOS launchd backend 保持行为。
- Linux systemd user、Windows logon task 先做设计或 dry-run shell。

不做：

- 不自动安装 Linux/Windows service。
- 不改变 `shuo app service` 用户可见语义。

验收：

- macOS service install/start/stop/restart/status 行为不变。
- CLI runtime boundary 不回退。
- stop timeout 和 shutdown ack 语义不变。

## Phase 5: Desktop Capability Boundary

目标：拆 hotkey、clipboard、text injection、active app、permissions 的平台边界。

范围：

- 保留 macOS CGEventTap 实现。
- 业务层只依赖 platform facade。
- 非 macOS 返回 capability-aware unsupported。
- 为 Windows/Linux backend 留小接口，不提前扩大。

不做：

- 不在第一步实现全局 hotkey。
- 不引入大型跨平台输入库，除非 PoC 证明必要。

验收：

- macOS suppress down/up 配对测试不回退。
- clipboard/paste 行为不变。
- profile route 仍使用 frontmost app 信息。

## Phase 6: Overlay Renderer Boundary

目标：共享 overlay model/layout/theme，renderer 平台化。

范围：

- 保留 macOS AppKit renderer。
- 抽 renderer availability/capability。
- 建立 Windows/Linux renderer 骨架。
- 确认 material fallback：liquid_glass -> blurred_glass -> translucent -> solid。

不做：

- 不一次性实现 Windows/Linux 完整 overlay。
- 不让 renderer 直接读取业务配置。

验收：

- macOS overlay 视觉和状态机不回退。
- `OverlayCmd`、`OverlayModel`、layout tests 通过。
- capability 能表达材质和窗口能力降级。

## Phase 7: Windows Overlay PoC

目标：验证 Windows 原生 overlay 技术路线。

范围：

- 验证 Win32 topmost/layered/tool window。
- 验证 DWM/Mica/Acrylic 或 fallback。
- 验证文字绘制、show/hide 延迟、鼠标穿透。

验收：

- 记录 Windows 11 结果。
- 记录 Windows 10 fallback。
- 若成本过高，修订 overlay 文档而不是硬做。

## Phase 8: Linux Wayland Overlay PoC

目标：验证 Wayland-first overlay 可用性。

范围：

- 验证主流 GNOME/KDE Wayland 能力。
- 验证 layer-shell 或可替代方案。
- 验证无法置顶/穿透/锚定时的 solid fallback。

验收：

- 核心状态和文本可读。
- 能力不足被清楚诊断。
- X11 backend 是否需要实现有明确结论。

## Phase 9: Tauri GUI Client PoC

目标：验证 GUI 作为独立 client 的开销和集成成本。

范围：

- 建一个最小 Tauri app。
- 连接 daemon IPC。
- 展示 status snapshot 和 history summary。
- 测量冷启动、空闲内存、空闲 CPU、包体。

不做：

- 不替代 TUI。
- 不加入 daemon 常驻路径。

验收：

- GUI 关闭后 daemon 继续运行。
- daemon 未打开 GUI 时不加载 WebView。
- 记录三端 PoC 指标。

## Phase 10: First Non-macOS Core Backend

目标：在不依赖完整 overlay/GUI 的前提下，让核心能力在第一个非 macOS 平台可运行。

建议顺序：

1. Linux cloud ASR core：config、ASR provider、post、history、IPC。
2. Linux service manager。
3. Linux desktop capability。
4. Windows core。

选择 Linux first 是为了更快复用 Unix socket 和 CI；Windows 设计约束必须在接口评审时同时考虑。

## 持续维护

- 每完成一个 phase，更新 `overview.md` 的阶段状态。
- 发现文档假设错误，先改文档，再改实现。
- 不把 PoC 临时日志放进长期文档；只记录结论、风险和决策。
