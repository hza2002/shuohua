# Cross-Platform Design

本目录是跨平台改造的长期设计入口。这里记录当前设计基线、已知风险、
开发顺序和需要保护的不变量。它不是不可修改的规范：开发过程中发现新事实时，
先更新对应文档，再调整实现计划。

## 阅读路由

| 要改 | 读 |
|---|---|
| 总体目标、阶段边界、文档维护规则 | [overview.md](overview.md) |
| macOS 回归基线、平台边界审计 | [macos-baseline.md](macos-baseline.md) |
| Windows 开发规范、路径、IPC、桌面能力、验证顺序 | [windows.md](windows.md) |
| Windows 首次实机/VM runtime smoke checklist | [windows-runtime-validation.md](windows-runtime-validation.md) |
| CLI / daemon / GUI / packaged app 的数据归属和路径根 | [app-data.md](app-data.md) |
| GUI App / Tauri client | [gui.md](gui.md) |
| 三端原生 overlay renderer | [overlay.md](overlay.md) |
| 一份配置和 theme 如何跨端工作 | [config-theme.md](config-theme.md) |
| 平台能力模型、降级和诊断 | [platform-capabilities.md](platform-capabilities.md) |
| IPC transport、单实例、service manager | [ipc-service.md](ipc-service.md) |
| 渐进开发顺序和验证门槛 | [development-plan.md](development-plan.md) |

## 文档规则

- 总览只写跨模块原则和依赖顺序，不塞每个模块的实现细节。
- 独立改造放独立文档；两个模块没有直接依赖时不要揉成一个计划。
- 每次实现跨平台阶段前，先更新对应文档里的当前判断、风险和验收标准。
- 文档里的技术选择是默认路线，不是锁死方案；PoC 或实现发现更好路径时可以修订。
- 只有对外数据契约、隐私边界和“macOS 不回退”是不变量。
- 已废弃、不再指导实现的方案才进 `docs/archive/`。
- `docs/superpowers/` 是本地 agent 计划区，默认不进 Git，不作为长期设计源。
