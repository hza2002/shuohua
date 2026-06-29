# overlay — 录音浮层（Liquid Glass）

**TL;DR**：主 NSPanel + profile picker 小 NSPanel；`NSGlassEffectView` 必须作内容的**兄弟节点**，不作 contentView（否则 AppKit 二次磨砂材质回退）；`OverlayCmd` 是 Tokio→AppKit，`OverlayAction` 是 AppKit→Tokio；command/model/layout 平台无关。

> **何时读**：改浮层视觉/动画/材质、Notice/Error 反馈、平台 renderer。
> **不在这里**：触发浮层的业务（voice/post/reload）在各自模块；i18n 文案见 [architecture](../architecture.md)。
> **代码**：`src/overlay/`。`command.rs`(OverlayCmd+State+Handle) / `model.rs`(OverlayModel+tick) / `layout.rs`(LayoutFrame+纯几何) 平台无关；`macos/`(view+chrome+icon_fx+debug) 是唯一 renderer。状态图标的自绘动效（光晕/雷达/跳点/彗星尾/电平条）都在 `macos/icon_fx.rs`，是纯视觉实现细节。

## 材质与视图层级（不变）

- 默认 glass variant **11=`bubbles`**（更"水珠"），备选 **19=`control`**（跟系统控件一致）；`[overlay.macos].glass_variant` 可覆写（调试用）。
- 私有方法 `set_variant:` 在 macOS 26.5 仍存在，selector 稳定。破坏时回退 `NSVisualEffectMaterialHUDWindow`。不上 App Store，私有 API 风险可控。
- 视图结构：`InteractivePanel(NSPanel subclass, borderless/透明/无阴影/level=NSStatusWindowLevel, canBecomeKeyWindow=false) → root NSView(圆角 mask) → NSGlassEffectView + 内容子视图 siblings`。profile picker 也走同一 panel/chrome 路径。
- 位置锚定 focused window 内部，配置只控垂直（top/middle/bottom），水平始终居中。macOS 26 不可用静默回退 HUDWindow，不弹错。

## OverlayCmd / OverlayAction（契约 owner）

上层只通过 `OverlayCmd` 驱动（`SetState`/`SetStats`/`SetLevel`/`SetApp`/`SetText`/`AppendSegment`/`ReplaceRecentSegments`/`Notice`/`Hide`/`Dismiss`/`ReloadConfig`/`Relabel`/`Quit`，定义见 `command.rs`）。`SetLevel{rms}` 是录音电平（高频 ~20/s，mailbox 按最新值覆盖、不进队列），驱动 Recording 电平条。`OverlayModel` 拥有所有时序状态（Notice/Error TTL、`pending_hide`、`recording_started→dur_ms`），`tick(now)->TickOutcome` 推进；`layout.rs` 返回纯几何 `LayoutFrame`，时长格式化也在这。

AppKit 交互只通过 `OverlayAction` 反向通知 daemon。当前只有 `BindProfile { bundle_id, profile }`：daemon runtime 写 `config.toml` 的 `[profile]` 路由并 `reload_now()`，其中 `profile` 必须是路由 id/文件 stem（如 `agent`），不是展示名。`SetApp.profiles` 是 daemon 从 profile 文件汇总出的轻量 `ProfileChoice { id, display_name, asr_provider, chain_summary }` snapshot，renderer 只渲染，不读配置文件；`display_name` 只用于 UI。AppKit callback 不写文件、不 await future，只做本地状态或 mpsc 非阻塞 send。

**文本高度不估算**：body 显示为 `NSScrollView + NSTextView`，面板视口高度按 `max_text_lines` 封顶，超出内容在 body 内部滚动。AppKit renderer 必须用同一个 `NSTextView/layoutManager` 测出内容真实高度、真实 line fragment 数、单行高度、后续行高增量、最后 `max_text_lines` 条 line fragment 的 viewport 高度和 scroll offset，再交给 `layout::body_geometry_with_tail_metrics`；`BodyGeometry` 是唯一权威，renderer 设置 panel height、body viewport、document frame、follow 滚动 offset、scroll indicator frame 都使用这个返回值。document frame 高度必须覆盖真实内容高度和 AppKit tail scroll extent（`scroll_offset + viewport_height`），否则安全 scroll offset 会被 clip view clamp 回去，最大行数外的上一行会露出。overflow 由真实 line count 判定，不由高度 cap 反推。别引入「字数×每行字数」、固定 `BODY_LINE_H` 多行推导或第二套 indicator 比例算法；`BODY_LINE_H` 只作初始/兜底单行基线。

body 布局数值分三类：`WIDTH/H_PAD/BOTTOM_PAD/HEADER_BODY_GAP/BASE_HEIGHT/BODY_LINE_H` 和 indicator 宽度/最小高度/短 fade 时长是设计常量；`textContainerInset`、`usedRectForTextContainer`、真实 line fragment 高度和数量、glyph/used bounding、最后 N 行 line fragment viewport/offset、document visible rect 是 AppKit 派生值；额外顶部遮罩、顶部 fade、status bar hint、固定像素 guard、字数估算行高、为滚动条缩窄正文宽度都不属于布局解法。

**overlay 只镜像 ASR 原文**：整个 recording 生命周期里，ASR 返回的所有事件（Partial/Segment/Final）都增量转发到 overlay——包括 finalize 阶段的 Partial。**不显示任何后处理链路内容**：Thinking 只改状态图标，LLM/post 结果直接上屏不推 overlay。

## 三条反馈通道

- **Notice**：meta 行（平时显示 `app · chain`）临时换黄字 warn，默认 3000ms 到点恢复（voice 侧常量 `NOTICE_TTL_MS`）。用于非阻断 warn（post step 失败/超时）。
- **Error**：text 区（平时显示 partial/final）换红字，盖住 partial/final（`display_text()` 优先级 error>final>segments+partial）。`ERROR_TTL_MS=5000`（比 notice 长，留用户读完决定重试）。
- **延期 Hide**：成功路径发 `Hide` 时若 notice 还活，设 `pending_hide`，等 notice 到期再隐藏；新 session 的 `SetState{Connecting}` 抢断 lingering；ESC 走 `Dismiss` 强制立刻关。避免 warn 一闪被 Hide 吞掉、Error 不被自动粘贴流程截胡。

## 交互与焦点闸

- 鼠标事件默认穿透；每帧按 `visible && (body_overflow || bundle_id.is_some())` 切 `setIgnoresMouseEvents`。
- 主 panel 和 picker panel 都是 `InteractivePanel`，`canBecomeKeyWindow=false`，`becomesKeyOnlyIfNeeded=true`，显示用 `orderFrontRegardless()`，不调用 `makeKeyAndOrderFront()`。
- body hover 进入时暂停自动跟随；离开时恢复 follow 并滚到底。body 正文不为滚动条让位；`NSTextContainer.lineFragmentPadding=0`，正文宽度与 body 内容区等宽，左右贴近同一 panel padding。滚动指示条在初始化时作为悬浮 layer 挂到 body 上，位于正文右侧的固定视觉间距后；frame 更新禁用隐式 Core Animation 动画，避免从旧位置滑入，opacity 使用短 fade 进入/退出。内容首次超过最大行数或用户滚动时，指示条短暂出现后隐藏；不做顶部/底部 fade 或 status bar。
- pipeline/meta 区域仅在当前会话有 bundle id 时可点。点击打开 profile picker，picker 只列出可替换 profile（不含当前 profile），点击行发送 `OverlayAction::BindProfile`；绑定下次会话生效。meta 长链路右对齐并从头部截断，保留右侧实际链路尾部；picker 按可选项宽度自适应、右侧与主 panel 右侧对齐、profile 前缀高亮。picker 关闭只看鼠标是否仍在 picker 内；离开 picker 800ms 后自动关闭，主 panel 不续期 picker。

## 平台边界

command/model/layout 平台无关；`macos/` 是唯一 renderer（`view.rs` NSPanel+mpsc+动画，`chrome.rs` 集中 glass/SkyLight SPI/HUD fallback，`debug.rs` SPI 探针）。`mod.rs` 用 `#[cfg(target_os="macos")] pub use macos::run`。**加新平台 = 加 `overlay/<platform>/` 兄弟目录写自己的 view/chrome + 加 `MacosOverlayCfg` 兄弟 struct，不动 command/model/layout/上层。**

## 本模块持有的不变量

- `NSGlassEffectView` 必须作子视图，不作 contentView（否则 AppKit 加 legibility blur）。
- AppKit 主线程与 tokio runtime 用 `tokio::sync::mpsc` 通信，**绝不在 AppKit callback 里 block tokio future**（用 `try_recv` 或 dispatch 到主线程）。
- AppKit 子类只放在 `macos/`，平台无关层只暴露命令/动作契约。
- body document height、viewport height、overflow、bottom scroll offset、scroll indicator frame 必须来自同一个 `BodyGeometry`；document height 必须覆盖 renderer 请求的 tail scroll extent；程序自动 follow 滚动不刷新 indicator discoverability。
