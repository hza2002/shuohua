# 归档

历史里程碑的设计文档和实施计划。**这里的文件不再代表当前事实**，只作为决策回溯参考。
`docs/archive/` 视为一个历史节点集合，不再按主题拆成多个权威源。

当前权威文档：

- [DESIGN.md](../DESIGN.md) — 技术设计、架构不变量
- [SCHEMA.md](../SCHEMA.md) — UDS 协议 + history.jsonl
- [MODULES.md](../MODULES.md) — 已实现模块和源码边界
- [CLI.md](../CLI.md) — CLI + launchd
- [CHANGELOG.md](../../CHANGELOG.md) — 阶段性历史和重要取舍

## 已归档

- **[M10.md](M10.md)** — Multi-session ASR + 本地 VAD 切 session 的设计说明。M10 已经接入主流程；配置语义、控制协议、history 字段含义现在以 [DESIGN.md §2.9](../DESIGN.md#29-客户端-vad--多段-session思考不计费机制) 和源码为准。
- **[M10_PLAN.md](M10_PLAN.md)** — M10 实施计划（任务拆分、伪代码示例、单测占位）。已完成；保留以备未来同类多 session 改动参考。
- **[CONFIGURE_ARCHITECTURE.md](CONFIGURE_ARCHITECTURE.md)** — Configure 重构阶段架构记录。当前配置模块边界以 [MODULES.md](../MODULES.md) 和源码为准。
- **[TUI_PLAN.md](TUI_PLAN.md)** — TUI 阶段改造计划。已实现内容以 [DESIGN.md](../DESIGN.md)、[MODULES.md](../MODULES.md) 和 [CHANGELOG.md](../../CHANGELOG.md) 为准。
- **[TODO.md](TODO.md)** — 早期任务清单。保留作历史参考；当前工作不要从这里取需求。
- **[superpowers/](superpowers/)** — agentic implementation plans 和 research specs。它们与其他归档文档一样，只记录过去的实施过程，不代表当前事实源。
