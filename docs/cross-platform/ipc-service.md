# IPC And Service

## IPC

JSON-line command/event protocol 保持共享。平台只替换 transport：

| 平台 | Transport |
|---|---|
| macOS | Unix domain socket |
| Linux | Unix domain socket |
| Windows | Named Pipe |

协议层不应依赖 transport path 类型。client/server 应通过 transport facade 创建 stream。

## 单实例

daemon 单实例锁和 stale endpoint 清理需要平台化：

- macOS/Linux：lock file + UDS endpoint。
- Windows：named mutex 或 lock file + Named Pipe endpoint。

stale endpoint 清理仍只允许 daemon 持 lock 的启动路径执行。

## Service Manager

Service manager 是用户会话级服务，不按 server daemon 设计：

| 平台 | 推荐 |
|---|---|
| macOS | launchd user agent |
| Linux | systemd user |
| Windows | logon task / startup app；慎用 Windows Service |

Windows Service 通常不适合需要用户会话、桌面权限、clipboard/text injection 的应用。

## Smart Fallback

CLI/TUI/GUI 连接 daemon 时：

1. probe endpoint。
2. 如果不存在，按平台策略尝试启动用户 daemon。
3. 等待 ready event 或超时。
4. 如果 endpoint stale，只允许 daemon lock owner 清理。

## 验收

- macOS 当前 JSON-line 协议不变。
- TUI/GUI/client 不关心 transport 细节。
- Windows backend 只需替换 transport，不 fork protocol。
- service status 输出使用相同 user-facing contract。
