# GUI App

## 决策

GUI App 使用 Tauri 最新稳定版。Tauri GUI 是按需 client，不嵌入 daemon，不参与录音热路径。
daemon 未打开 GUI 时不加载 WebView。

Tauri 当前文档定位是 Rust backend + Web frontend，底层通过 WRY 使用各平台 WebView，
并提供 commands/events 与前端通信；bundler 覆盖 macOS、Windows、Linux。这个能力匹配
配置、历史、诊断、onboarding 等复杂 GUI。

## 范围

第一阶段 GUI 覆盖 TUI 的主要能力：

- Status：daemon 状态、当前 session、audio meter、ASR/post 概览。
- History：分页、搜索、详情、audio 关联、统计和图表。
- Configure：查看配置、编辑入口、模板/向导、validate/reload。
- Diagnostics：doctor、权限、服务状态、支持 bundle 入口。
- Service：install/start/stop/restart/status。

后续 GUI 可以扩展：

- 登录和账号状态。
- 更复杂的配置编辑器。
- 更强的历史浏览、筛选、导出。
- 首次启动 onboarding。

## 非目标

- GUI 不替代 daemon。
- GUI 不负责全局 hotkey、录音、ASR、post、history 落盘。
- GUI 不要求常驻；关闭 GUI 不影响 daemon。
- GUI 不作为 overlay 的默认技术方案。overlay 是否复用 Web 技术要由 overlay PoC 决定。

## 通信边界

GUI 应使用与 TUI 同级的 client API。短期可以复用 JSON-line IPC；长期可以在 daemon 侧
增加适合 GUI 的 command/event，但不得绕过 schema/history/state 的 ownership。

Tauri command 只做 GUI 进程内桥接：

```text
frontend -> Tauri command -> shuohua client API -> daemon IPC
daemon event -> shuohua client API -> Tauri event -> frontend
```

## 验收指标

GUI PoC 进入实现前必须记录：

- macOS/Windows/Linux 打包体积。
- 冷启动时间。
- 打开 GUI 后空闲内存。
- 空闲 CPU。
- 与 daemon 连接断开/重连行为。

这些指标用于决定 GUI 是否可选打包、是否默认安装，以及是否需要 lazy-load 大型页面。
