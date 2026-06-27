# overlay — 录音浮层（Liquid Glass）

**TL;DR**：单 NSPanel；`NSGlassEffectView` 必须作内容的**兄弟节点**，不作 contentView（否则 AppKit 二次磨砂材质回退）；`OverlayCmd` 是上层唯一接口；command/model/layout 平台无关。

> **何时读**：改浮层视觉/动画/材质、Notice/Error 反馈、平台 renderer。
> **不在这里**：触发浮层的业务（voice/post/reload）在各自模块；i18n 文案见 [architecture](../architecture.md)。
> **代码**：`src/overlay/`。`command.rs`(OverlayCmd+State+Handle) / `model.rs`(OverlayModel+tick) / `layout.rs`(LayoutFrame+纯几何/文本) 平台无关；`macos/` 是 AppKit/Liquid Glass renderer，`windows.rs` + `windows/direct2d.rs` 是 Win32 baseline renderer。

## 材质与视图层级（不变）

- 默认 glass variant **11=`bubbles`**（更"水珠"），备选 **19=`control`**（跟系统控件一致）；`[overlay.macos].glass_variant` 可覆写（调试用）。
- 私有方法 `set_variant:` 在 macOS 26.5 仍存在，selector 稳定。破坏时回退 `NSVisualEffectMaterialHUDWindow`。不上 App Store，私有 API 风险可控。
- 视图结构：`NSPanel(borderless/透明/无阴影/level=NSStatusWindowLevel) → root NSView(圆角 mask) → NSGlassEffectView + 内容子视图 siblings`。
- 位置锚定 focused window 内部，配置只控垂直（top/middle/bottom），水平始终居中。macOS 26 不可用静默回退 HUDWindow，不弹错。

## OverlayCmd（契约 owner）

上层只通过 `OverlayCmd` 驱动（`SetState`/`SetStats`/`SetLevel`/`SetApp`/`SetText`/`AppendSegment`/`ReplaceRecentSegments`/`Notice`/`Hide`/`Dismiss`/`ReloadConfig`/`Relabel`/`Quit`，定义见 `command.rs`）。`SetLevel{rms}` 是录音电平（高频 ~20/s，mailbox 按最新值覆盖、不进队列），驱动 Recording 电平条。`OverlayModel` 拥有所有时序状态（Notice/Error TTL、`pending_hide`、`recording_started→dur_ms`），`tick(now)->TickOutcome` 推进；`layout.rs` 返回纯几何 `LayoutFrame`，文本截断/行数/时长格式化也在这。

## 三条反馈通道（全复用主 panel，不开第二个）

- **Notice**：meta 行（平时显示 `app · chain`）临时换黄字 warn，默认 3000ms 到点恢复（voice 侧常量 `NOTICE_TTL_MS`）。用于非阻断 warn（post step 失败/超时）。
- **Error**：text 区（平时显示 partial/final）换红字，盖住 partial/final（`display_text()` 优先级 error>final>segments+partial）。`ERROR_TTL_MS=5000`（比 notice 长，留用户读完决定重试）。
- **延期 Hide**：成功路径发 `Hide` 时若 notice 还活，设 `pending_hide`，等 notice 到期再隐藏；新 session 的 `SetState{Connecting}` 抢断 lingering；ESC 走 `Dismiss` 强制立刻关。避免 warn 一闪被 Hide 吞掉、Error 不被自动粘贴流程截胡。

## 平台边界

command/model/layout 平台无关；`renderer.rs` 拥有平台 renderer 选择，`mod.rs` 的 `run()` 只转发到 renderer。macOS renderer 使用 NSPanel/AppKit glass；Windows 当前 renderer 是 Win32 no-activate layered window + Direct2D/DirectWrite per-pixel baseline，GDI 保留为 fallback。**加新平台 = 加 `overlay/<platform>/` 兄弟目录写自己的 view/chrome + 平台配置 struct，不动 command/model/layout/上层。**

Windows baseline renderer 是当前可用 fallback，不是最终视觉路线。它避免 Tauri/WebView 进入 daemon 热路径，但 `UpdateLayeredWindow` + DIB + 手绘 shadow 的质感上限低于 macOS Liquid Glass；最终 Windows renderer 应保留 Win32 no-activate/topmost shell，单独评估 DirectComposition / Windows Composition + DirectWrite/Direct2D 的现代路线。Windows App SDK Mica/Acrylic 只能作为材质候选，不应在没有 rounded clipping、shadow、solid text 和 animation PoC 前直接接入 daemon overlay。

Windows 状态图标路线已经从手绘 primitive 转向系统 icon font：`Segoe Fluent Icons` 优先，
`Segoe MDL2 Assets` fallback。当前 Direct2D fallback 只渲染静态 glyph；后续 Composition backend
负责 opacity/scale/rotate/translate 动画，不自绘 icon 本体。

Windows Composition backend 目前仍是 `SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_PROBE` gated probe：已能创建
DirectComposition visual tree、绑定/resize panel surface，并用 Direct2D-on-DXGI-surface 绘制圆角半透明
panel、系统 icon glyph 和文本；还验证了 compositor-owned rounded clipping 和 panel opacity binding。
Composition probe 与 Direct2D fallback 共用 shadow outset geometry：surface 包含 renderer-owned outset，
panel/content 坐标保持 inset。Composition shadow surface 已绑定到独立 `shadow` visual，用于验证分层 plumbing。
Icon visual 已绑定静态 opacity animation probe，用于验证 composition static animation binding。
Composition probe 会按状态 icon plan 切换 state-driven opacity animation；transform/scale/rotate 动画仍未实现。
默认可见 renderer 仍是 Direct2D per-pixel fallback，最终 shadow/material/animation 和默认 backend 切换尚未完成。

`renderer.rs` 也持有 renderer capability skeleton：静态描述当前 renderer 是否可用、材质降级、
置顶、输入穿透和窗口锚定状态。它复用 `platform::capability` 的 status 类型，不执行窗口创建、
权限 probe 或业务配置读取；macOS 现有 AppKit 行为不因此改变。

## 本模块持有的不变量

- `NSGlassEffectView` 必须作子视图，不作 contentView（否则 AppKit 加 legibility blur）。
- AppKit 主线程与 tokio runtime 用 `tokio::sync::mpsc` 通信，**绝不在 AppKit callback 里 block tokio future**（用 `try_recv` 或 dispatch 到主线程）。
