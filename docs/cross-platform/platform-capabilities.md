# Platform Capabilities

## 目标

平台差异需要显式建模。调用方不应通过字符串错误猜测某能力是否支持，也不应把
macOS 的实现细节投射到 Windows/Linux。

## 状态模型

当前建议共享状态：

| 状态 | 含义 |
|---|---|
| `available` | 当前环境可用 |
| `unsupported` | 该平台/backend 不支持 |
| `unavailable` | 理论支持，但当前缺权限、依赖或运行时条件 |
| `partial` | 可用但有明确限制 |
| `degraded` | 已启用 fallback |
| `unknown` | probe 失败，无法确定 |

每个状态应携带：

- capability id。
- platform/backend。
- human-readable summary。
- machine-readable reason code。
- optional next step。

## 能力列表

第一批候选能力：

- `ipc.transport`
- `daemon.single_instance`
- `service.manager`
- `process.probe`
- `desktop.hotkey`
- `desktop.hotkey_suppression`
- `desktop.clipboard`
- `desktop.text_injection`
- `desktop.active_app`
- `desktop.permissions`
- `overlay.renderer`
- `overlay.material`
- `overlay.always_on_top`
- `overlay.input_passthrough`
- `overlay.window_anchor`
- `audio.capture`
- `audio.convert`
- `path.open_reveal`

## 消费方

- daemon startup：决定启用哪些 backend。
- doctor：展示问题和 next step。
- TUI/GUI：展示当前环境能力和降级。
- history/trace：记录低频诊断，不记录敏感正文。

## 设计约束

- capability probe 不执行高风险动作。
- probe 不应阻塞 AppKit/Tauri/window callback。
- permission 诊断应平台化。
- unsupported 是正常状态，不是 panic。
