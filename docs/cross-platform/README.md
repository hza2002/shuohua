# Cross-Platform Design

本目录是跨平台改造的长期设计入口。这里记录会影响后续实现顺序、
模块边界和对外契约的决定；临时调研、PoC 日志和废弃方案不放这里。

## 阅读路由

| 要改 | 读 |
|---|---|
| 总体目标、阶段边界、文档维护规则 | [overview.md](overview.md) |
| GUI App / Tauri client | [gui.md](gui.md) |
| 三端原生 overlay renderer | [overlay.md](overlay.md) |
| 一份配置和 theme 如何跨端工作 | [config-theme.md](config-theme.md) |
| 平台能力模型、降级和诊断 | [platform-capabilities.md](platform-capabilities.md) |
| IPC transport、单实例、service manager | [ipc-service.md](ipc-service.md) |

## 文档规则

- 总览只写跨模块原则和依赖顺序，不塞每个模块的实现细节。
- 独立改造放独立文档；两个模块没有直接依赖时不要揉成一个计划。
- 每次实现跨平台阶段前，先更新对应文档里的契约和验收标准。
- 已废弃、不再指导实现的方案才进 `docs/archive/`。
- `docs/superpowers/` 是本地 agent 计划区，默认不进 Git，不作为长期设计源。
