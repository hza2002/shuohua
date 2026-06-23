# GUI App

## 当前设计基线

GUI App 使用 Tauri 最新稳定版。Tauri GUI 是按需 client，不嵌入 daemon，不参与录音热路径。
daemon 未打开 GUI 时不加载 WebView。

Tauri 当前文档定位是 Rust backend + Web frontend，底层通过 WRY 使用各平台 WebView，
并提供 commands/events 与前端通信；bundler 覆盖 macOS、Windows、Linux。这个能力匹配
配置、历史、诊断、onboarding 等复杂 GUI。

如果后续 PoC 证明 Tauri 在目标平台的启动、内存、打包或系统集成成本不可接受，可以重新评估。
在有反证前，Tauri 是默认路线。

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

Phase 9b 先建立共享 client API 边界，不创建 Tauri workspace：

- client API 只封装 daemon 连接和既有 `ipc::protocol::Command` / `Event`，不新增 JSON-line
  wire shape，不 bump `PROTO_VERSION`。
- TUI 和后续 GUI backend 都应把它当作 daemon client 入口；GUI frontend 不能直接构造
  transport、读写 socket 或读取 history/config 文件。
- client API 可以提供 GUI 首屏需要的命令组，例如 daemon status、subscribe、history page 和
  history stats，但这些命令必须映射到现有 IPC protocol。
- client API 不依赖 Tauri、WRY、WebView、frontend build output 或 window runtime；daemon、
  TUI 和 shared client API 都不得链接 Tauri/WebView。
- Tauri command 只调用 client API，并把结果转换为前端 view model；view model 的本地化和展示
  细节留在 GUI 层，不进入 daemon protocol。

## 验收指标

GUI PoC 进入实现前建议记录：

- macOS/Windows/Linux 打包体积。
- 冷启动时间。
- 打开 GUI 后空闲内存。
- 空闲 CPU。
- 与 daemon 连接断开/重连行为。

这些指标用于决定 GUI 是否可选打包、是否默认安装，以及是否需要 lazy-load 大型页面。

## Phase 9a Tauri GUI PoC Baseline

Phase 9a 先记录 Tauri GUI PoC 基线，不写 GUI app，不把 WebView 放进 daemon。
当前依据 Tauri v2 官方文档的判断：

- Tauri 仍应作为独立按需 client。`shuo daemon` 不链接 Tauri runtime、不创建 WebView、不运行
  frontend build step；GUI 关闭后 daemon 继续独立运行。
- Tauri command/event 只做 GUI 进程内桥接。前端通过 Tauri command 调用 GUI backend，
  GUI backend 再使用 shuohua client API 连接 daemon IPC；daemon event 通过 GUI backend
  转成 Tauri event 给前端。
- Tauri v2 permissions/capabilities 必须纳入 PoC。只给主 window 授权需要的 command/plugin，
  不打开 shell/filesystem/http 等宽权限；如需外部打开配置目录，优先复用已有 client API 或
  明确 scope。
- sidecar/external binary 不是默认路线。Tauri 文档支持 `bundle.externalBin` 和 shell plugin
  管理 sidecar，但 shuohua 的 daemon 已有 service lifecycle；GUI PoC 不应把 daemon 常驻进程
  变成 GUI 子进程。只有安装包分发需要同捆二进制时，才评审 sidecar。
- `tauri build`/`tauri bundle` 支持 release build、bundle 选择、target triple 和平台配置合并；
  PoC 指标应基于 release build 记录，而不是 dev server。
- 前端第一屏只做实际 client 功能：Status snapshot、History summary、Diagnostics summary。
  不做营销首页，不做配置编辑器完整版。

Phase 9 PoC checklist：

- 进程边界：daemon 未启动 GUI 时无 WebView/Tauri 进程；GUI 退出不停止 daemon。
- IPC：GUI 能连接现有 daemon transport，展示 status snapshot 和 history summary；daemon
  不在线时显示可恢复错误并支持重连。
- 安全：列出 Tauri capabilities/permissions 文件，确认未启用无关 shell/filesystem/http 权限。
- 指标：macOS/Windows/Linux release bundle size、cold start time、open GUI idle RSS、
  open GUI idle CPU、连接 daemon 首次数据时间。
- 打包：记录 `tauri build` 和 `tauri bundle` 产物路径、bundle 类型和签名/未签名状态。
- 回退：TUI 继续可用；GUI PoC 不改变 CLI/TUI 命令和 JSON-line IPC protocol。

Phase 9b 验收：

- 存在共享 daemon client API 边界，TUI 至少通过该边界获取 `IpcClient` 类型。
- `Cargo.toml` 不新增 Tauri/WRY/WebView 依赖。
- `src/daemon/**`、`src/tui/**` 和共享 client API 不出现 Tauri/WRY/WebView token。
- IPC protocol round-trip 测试继续通过，`PROTO_VERSION` 不变。

参考资料：

- [Tauri v2 develop guide](https://v2.tauri.app/develop/)
- [Tauri JavaScript Window API](https://v2.tauri.app/reference/javascript/api/namespacewindow/)
- [Tauri CLI build command](https://v2.tauri.app/reference/cli/#build)
- [Tauri CLI bundle command](https://v2.tauri.app/reference/cli/#bundle)
- [Tauri sidecar guide](https://v2.tauri.app/learn/sidecar-nodejs/)
- [Tauri v2 permissions system](https://v2.tauri.app/blog/tauri-20/#the-allowlist-is-dead-long-live-the-allowlist)
