# Cross-Platform Overlay

## 当前设计基线

Overlay 三端优先原生 renderer。共享 `OverlayCmd`、model、layout、theme token 和状态语义；
平台 renderer 只负责窗口能力、材质、绘制、动画、定位和输入穿透。

这个方向允许随 PoC 修订。只要共享 command/model/theme 不被破坏，具体 renderer 技术可以换。

## 视觉优先级

材质按能力降级：

1. `liquid_glass`：液态玻璃，优先使用平台提供或可维护实现。
2. `blurred_glass`：普通半透明 + background blur。
3. `translucent`：半透明 tint，无 blur。
4. `solid`：实心背景，保证文字可读。

可读性优先于特效。任何平台上，如果 blur/透明导致对比度不足或实现成本过高，应降级到
更重 tint 或 solid。

## 平台 renderer

### macOS

保留当前 AppKit renderer：

- `NSPanel` borderless / nonactivating / top-level。
- `NSGlassEffectView` 优先 Liquid Glass。
- SPI 不可用时 fallback 到 HUD/blur/tint。
- 位置锚定 focused window，失败时退屏幕位置。

### Windows

优先 Windows 11。推荐方向：

- Win32 原生 overlay window。
- DWM backdrop / Acrylic / Mica 能用则用。
- 绘制层候选：Direct2D、Skia、softbuffer/wgpu；选择前先 PoC。
- 不支持高级材质时退 translucent/solid。

Windows 10 可运行，但高级材质不是必须。

### Windows Phase 7a PoC Baseline

Phase 7a 先记录 Windows overlay 技术路线，不写 backend。当前依据 Microsoft 文档的判断：

- 窗口形态优先 Win32 borderless popup/top-level window。`CreateWindowEx` 支持创建带 extended
  style 的 overlapped、popup 或 child window；overlay PoC 应以 popup/top-level 为主，避免
  先绑定到 GUI/WebView 宿主。
- 基础样式候选：`WS_EX_TOPMOST` 保持置顶，`WS_EX_TOOLWINDOW` 避免进入 Alt-Tab/taskbar，
  `WS_EX_NOACTIVATE` 避免抢焦点，`WS_EX_LAYERED` 支持 alpha/layered 绘制。Microsoft
  extended window styles 文档明确 layered window 由 `WS_EX_LAYERED` 表达；Windows 8 起
  top-level 和 child window 都支持 layered style。
- 置顶和定位用 `SetWindowPos` 验证。Microsoft 文档把 topmost window 描述为 Z-order 中最高
  rank；PoC 需要确认录音期间不会被普通 app 覆盖。
- 透明/半透明先走 `SetLayeredWindowAttributes` 或 `UpdateLayeredWindow`，再评估
  Direct2D/DirectComposition。DirectComposition 可组合 layered window surface，但第一步不把
  GPU composition 作为必需项。
- Mica/DWM system backdrop 只作为 Windows 11 高级材质候选，不作为 baseline。Microsoft
  Mica 文档将 Mica 定位为 app/settings 等 long-lived window 背景；DWM system backdrop 文档
  也说明 DWM 可能按 heuristics 不绘制 backdrop。因此 overlay baseline 应先保证
  `translucent`/`solid` 可读，再探索 Acrylic/Mica。
- 鼠标穿透必须单独验证。`WM_NCHITTEST` 是系统决定 mouse message 去向的机制；PoC 可测试
  返回 `HTTRANSPARENT` 的区域穿透，同时记录 touch/pen 行为是否不同。
- 可选隐私能力：`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` 可让窗口不出现在 capture
  中，但这是后续 capability，不作为 overlay 可用性的 gate。

Phase 7 PoC 验收数据应写回本节：

- Windows 11：topmost、no-activate、tool window、alpha、click-through、文字绘制、
  show/hide 延迟、CPU/GPU 空闲占用、capture exclusion 是否可用。
- Windows 10：至少验证 layered translucent/solid fallback、topmost、no-activate、
  click-through；高级材质可 unsupported/degraded。
- 结论必须映射回 capability：`overlay.renderer`、`overlay.material`、
  `overlay.always_on_top`、`overlay.input_passthrough`、`overlay.window_anchor`。

参考资料：

- [CreateWindowExA function](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexa)
- [Extended Window Styles](https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles)
- [SetWindowPos function](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos)
- [SetLayeredWindowAttributes function](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setlayeredwindowattributes)
- [UpdateLayeredWindow function](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow)
- [WM_NCHITTEST message](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-nchittest)
- [Mica material](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)
- [DWM_SYSTEMBACKDROP_TYPE enum](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_systembackdrop_type)
- [SetWindowDisplayAffinity function](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)

### Windows Phase 7b Backend Skeleton

Phase 7b 的下一步是实现最小 Windows overlay backend skeleton，而不是完整视觉效果：

- 在 `overlay::renderer` 保持现有 facade，新增 Windows backend 文件和 cfg-gated 入口。
- 第一版 Windows backend 可以先返回 structured unsupported/degraded 或只提供可编译 skeleton；
  不引入 GUI/WebView，不改变 macOS AppKit renderer。
- 若引入 `windows` crate，必须仅限 Windows cfg 依赖路径；root daemon hot path 和 macOS build 不应
  链接 Windows-only API。
- 验收重点是模块边界、capability reporting、编译门槛和 macOS 不回退；真实 Windows 视觉 PoC
  需要在 Windows 11/10 机器上单独验证。

Phase 7b implementation status:

- `src/overlay/windows.rs` 已建立 cfg-gated backend skeleton。
- `overlay::renderer` 在 Windows target 下调度到 `windows::run()` 和
  `windows::renderer_capabilities()`。
- 当前能力报告均为 structured unsupported，backend `win32_overlay_skeleton`，reason
  `backend_skeleton_only`，用于 TUI/doctor 后续明确说明“backend skeleton 已存在但视觉效果未实现”。
- 不引入 `windows` crate，不创建窗口，不进入 daemon 热路径，不依赖 GUI/WebView。
- 在 macOS 主机 cross-check Windows target 时，当前先被既有 Unix-only `ipc::transport` 阻断；
  overlay skeleton 的真实 Windows 编译和视觉验证需要等 Named Pipe transport 或 Windows 环境可用后继续。

### Windows Phase 10ao Minimal Backend

Phase 10ao replaces the skeleton with a minimal native Win32 backend:

- The renderer creates one `WS_POPUP` window with `WS_EX_LAYERED`, `WS_EX_TOPMOST`,
  `WS_EX_TOOLWINDOW`, and `WS_EX_NOACTIVATE`.
- The first backend uses only Win32/GDI: translucent layered-window background, basic text drawing, show/hide,
  and `OverlayCmd::Quit` handling. It does not introduce Tauri, WebView, Direct2D, Skia, or wgpu.
- The backend reuses shared `OverlayModel` and layout/text helpers. Windows-specific code owns only the message
  pump, window creation, visibility, hit testing, and drawing.
- Hit testing returns `HTTRANSPARENT`, but mouse/touch/pen passthrough still needs real foreground-app validation.
- Anchoring is screen-only. Focused-window anchoring remains a later phase even though foreground process identity
  diagnostics exist.
- Capability is partial/degraded: renderer/topmost/input passthrough are smoke-only partial; material is degraded
  translucent fallback only; window anchor is degraded screen-only.

This phase does not validate audio, hotkey-triggered recording, clipboard/paste, advanced material, multi-monitor
behavior, fullscreen apps, UAC prompts, or final visual quality.

### Windows Phase 10ap DPI And Font Baseline

Phase 10ap fixes the first visual correctness layer before material polish:

- The renderer now scales window size, placement, text rectangles, and GDI font sizes from shared logical layout
  units to physical pixels using the current window DPI.
- Placement uses the Windows work area instead of raw primary-screen bounds, so the overlay avoids the taskbar in
  the common single-monitor case.
- Windows text uses the platform UI font path (`Segoe UI`) at DPI-scaled point sizes. This is still a GDI baseline,
  not the final text renderer.
- macOS does not hard-require JetBrains Mono or bundled SF Pro; the current AppKit renderer uses
  `NSFont::systemFontOfSize` / `boldSystemFontOfSize`.
- Do not bundle SF Pro. If a monospace or branded fallback becomes necessary, use an optional font with suitable
  redistribution terms as a fallback, not as a hard runtime dependency.

Remaining gates: per-monitor work area on secondary displays, DirectWrite/Direct2D text quality, rounded/shadowed
surface polish, fullscreen/UAC behavior, and final multi-monitor visual QA.

### Windows Phase 10aq Rounded GDI Baseline

Phase 10aq keeps the renderer native Win32/GDI, but fixes the most visible shape mismatch:

- The overlay applies the shared `overlay.surface.corner_radius` to the actual Win32 window region via
  `CreateRoundRectRgn` / `SetWindowRgn`.
- The layered-window alpha now uses the shared `overlay.surface.background_alpha` instead of a Windows-only fixed
  opacity.
- GDI font creation requests `CLEARTYPE_QUALITY` for the `Segoe UI` baseline.

This is still not the final text/material renderer. If Windows text remains visibly softer than system UI, the next
quality step should be a DirectWrite/Direct2D renderer foundation, not more GDI tuning. Shadow, Acrylic/Mica,
animation, focused-window anchoring, fullscreen/UAC behavior, and multi-monitor visual QA remain open.

### Windows Phase 10ar Direct2D/DirectWrite Foundation

Phase 10ar moves the renderer-quality foundation to the modern Windows 2D stack:

- The Win32 overlay window shell remains unchanged: popup, layered, topmost, tool window, no-activate, and
  hit-test passthrough stay owned by `src/overlay/windows.rs`.
- Direct2D/DirectWrite live in a Windows-only renderer module and do not leak into shared overlay model/layout,
  daemon runtime, IPC, hotkey, audio, clipboard, or paste code.
- The first renderer uses `ID2D1HwndRenderTarget` plus DirectWrite `IDWriteTextFormat` for text. This follows the
  stable desktop Direct2D/DirectWrite path and avoids adding DirectComposition/D3D/DXGI device ownership before it
  is needed.
- Existing GDI drawing stays as a fallback when Direct2D/DirectWrite initialization or painting fails.

This phase is a text and rounded-surface foundation, not a full material system. `UpdateLayeredWindow` per-pixel
surfaces, DirectComposition, Acrylic/Mica, shadows, animation, and capture-exclusion policy remain separate phases.
Manual visual QA is still required before upgrading capabilities.

### Windows Phase 10as Per-Pixel Layered Surface

Phase 10as fixes the next clarity issue found in manual QA: the previous Direct2D path still used
`SetLayeredWindowAttributes` global alpha, so Windows composited both the translucent background and the text at the
same opacity.

- The Direct2D renderer now draws into a top-down 32bpp DIB section through `ID2D1DCRenderTarget` /
  `CreateDCRenderTarget` + `BindDC`.
- The window is updated with `UpdateLayeredWindow` and `AC_SRC_ALPHA`, with `SourceConstantAlpha: 255`. Background
  pixels carry `overlay.surface.background_alpha`; text is rendered as solid 255-alpha text.
- The Win32 shell remains the same: popup, layered, topmost, tool window, no-activate, and hit-test passthrough are
  still owned by `src/overlay/windows.rs`.
- GDI fallback remains available and may still use global layered-window alpha when Direct2D/per-pixel setup fails.

This is the correct foundation before evaluating Acrylic/Mica/DirectComposition: material blur cannot make text sharp
if the whole window is globally alpha-composited. It still is not a complete Liquid Glass equivalent; native backdrop,
shadow, animation, focused-window anchoring, fullscreen/UAC behavior, and multi-monitor visual QA remain open.

### Linux

Wayland-first。X11 只保留 backend 接口位置，成本过高时允许 unsupported。

Wayland renderer 目标：

- 优先 compositor 支持的 overlay/layer-shell 类能力。
- 支持不了置顶/穿透/精确锚定时，降级到普通半透明/solid 状态窗。
- 核心录音、文本、状态、错误提示必须可用。

X11 backend 不作为第一阶段目标。

### Linux Phase 8a PoC Baseline

Phase 8a 先记录 Linux Wayland overlay 技术路线，不写 backend。当前依据 Wayland protocol
文档、wlr layer-shell protocol、KDE/GTK layer-shell 项目文档和 GNOME Mutter 公开 issue 的判断：

- Wayland core/xdg-shell 不提供普通 client 任意置顶、全局覆盖、鼠标穿透或前台窗口锚定能力。
  Wayland renderer 不能假设可以复刻 macOS `NSPanel` 或 Windows topmost window。
- 第一候选是 `wlr-layer-shell-unstable-v1`。该 protocol 为 desktop shell components 提供
  layer surface role，可设置 layer、screen edge/corner anchor、exclusive zone、margin 和
  keyboard interactivity。PoC 应优先验证 overlay/top layer、无 exclusive zone、固定屏幕锚定
  和不请求 keyboard focus。
- 支持矩阵必须实测。GTK Layer Shell 项目文档将 wlroots compositors、KDE Plasma Wayland 和
  部分 Mir compositors列为支持，将 GNOME-on-Wayland 和 X11 列为不支持；GNOME Mutter 的
  layer-shell issue 也表明 GNOME Shell/Mutter 对 wlr layer-shell 不是通用 client API。
- KDE Plasma 可作为主流桌面验证目标之一，但 KDE 的 `org_kde_plasma_shell` 文档明确是 shell
  内部实现细节，普通 client 不应依赖该私有 protocol。PoC 优先验证 wlr layer-shell 或
  toolkit wrapper，不走 KDE 私有 shell protocol。
- input passthrough 需要谨慎定义。layer-shell 能配置 keyboard interactivity，但鼠标穿透、
  pointer focus 和 click-through 行为取决于 compositor/toolkit；Phase 8 PoC 必须把
  `overlay.input_passthrough` 单独记录为 available/partial/unsupported。
- window anchor 在 Wayland 上默认不可用。若没有安全的 foreign-toplevel/activation 路线，
  Linux baseline 应先映射为 `screen_anchor` available、`window_anchor` unsupported/degraded。
- material baseline 是 `solid` 或 `translucent`。不要把 compositor blur 当作必需能力；
  blur/transparency 只有在具体 compositor/toolkit 可稳定验证后才上调。
- X11 fallback 只作为后续决策点。若 Wayland GNOME 无法支持 overlay，而 X11 能以 override-redirect
  或 EWMH route 工作，也要单独评估安全、焦点和维护成本，不能自动把 X11 作为第一 backend。

Phase 8 PoC 验收数据应写回本节：

- wlroots/Sway 或同类 compositor：layer-shell 是否可绑定、top/overlay layer 是否稳定、alpha
  是否可用、click-through 是否可实现、CPU/GPU 空闲占用、show/hide 延迟。
- KDE Plasma Wayland：wlr layer-shell/toolkit wrapper 是否可用，top layer、no focus、
  alpha、click-through 和多显示器表现。
- GNOME Wayland：记录 layer-shell 不可用时的 fallback 形态；若只能普通 xdg-shell window，
  capability 应清楚标为 degraded/unsupported。
- X11：只记录是否值得进入后续 PoC；不在 Phase 8a 承诺实现。
- 结论必须映射回 capability：`overlay.renderer`、`overlay.material`、
  `overlay.always_on_top`、`overlay.input_passthrough`、`overlay.window_anchor`。

参考资料：

- [Wayland core protocol](https://wayland.app/protocols/wayland)
- [XDG shell protocol](https://wayland.app/protocols/xdg-shell)
- [wlr layer shell protocol](https://wayland.app/protocols/wlr-layer-shell-unstable-v1)
- [GTK Layer Shell supported desktops](https://github.com/wmww/gtk-layer-shell)
- [KDE LayerShellQt](https://github.com/KDE/layer-shell-qt)
- [KDE plasma shell protocol warning](https://wayland.app/protocols/kde-plasma-shell)
- [GNOME Mutter layer-shell issue](https://gitlab.gnome.org/GNOME/mutter/-/issues/973)

### Linux Phase 8b Backend Skeleton

Phase 8b 的下一步是实现最小 Linux overlay backend skeleton，而不是承诺所有 compositor：

- 在 `overlay::renderer` 保持现有 facade，新增 Linux backend 文件和 cfg-gated 入口。
- 第一版 Linux backend 优先表达 Wayland capability/fallback，不急着绑定具体 layer-shell crate；
  可先返回 structured unsupported/degraded，确保 TUI/daemon 在 Linux 上能明确诊断 overlay 能力。
- 不把 GNOME/KDE/wlroots 的私有或不稳定假设写进共享层；真实 layer-shell / X11 backend 需要后续
  PoC 和目标环境验证。
- 验收重点是模块边界、capability reporting、非 macOS 编译路径和 macOS 不回退。

Phase 8b implementation status:

- `src/overlay/linux.rs` 已建立 cfg-gated backend skeleton。
- `overlay::renderer` 在 Linux target 下调度到 `linux::run()` 和
  `linux::renderer_capabilities()`。
- 当前能力报告使用 backend `wayland_overlay_skeleton`。renderer/material/top/input passthrough
  为 structured unsupported；`overlay.window_anchor` 为 degraded，reason `screen_anchor_expected`，
  明确 Wayland 上不承诺 focused-window anchoring。
- 不引入 layer-shell、GTK/KDE/Wayland crate，不创建窗口，不依赖 GUI/WebView。
- 在 macOS 主机 cross-check Linux target 时，当前先被 OpenSSL cross sysroot 阻断；真实 Linux
  编译和 compositor 验证需要 Linux 环境或配置好的 cross sysroot。

## Capability 分级

Renderer 启动时建议报告能力：

| Capability | 含义 |
|---|---|
| `visual_material` | liquid_glass / blurred_glass / translucent / solid |
| `always_on_top` | 能否稳定置顶 |
| `input_passthrough` | overlay 是否不抢鼠标输入 |
| `window_anchor` | 能否锚定当前前台窗口 |
| `screen_anchor` | 能否锚定屏幕固定位置 |
| `transparency` | 是否支持透明背景 |
| `blur` | 是否支持背景模糊 |
| `animation` | 是否支持低 CPU 动画 |

doctor/TUI/GUI 后续应能展示这些能力和降级原因。

Phase 6b 先把 renderer capability 作为只读 skeleton 放在 `overlay::renderer`：

- 使用 `platform::capability::{CapabilityStatus, CapabilityStatusKind, CapabilityId}`，
  不新增另一套 status 语义。
- `renderer_capabilities()` 返回 overlay renderer 相关能力的静态快照；它不创建窗口、
  不 probe 权限、不读取业务配置。
- macOS snapshot 只描述现有 AppKit renderer：`overlay.renderer` available、
  `overlay.material` degraded、`overlay.always_on_top` available、
  `overlay.input_passthrough` partial、`overlay.window_anchor` degraded。
- 非 macOS snapshot 先返回 structured unsupported，原因仍是 backend 未实现。
- material preference 使用固定降级顺序
  `liquid_glass -> blurred_glass -> translucent -> solid`；Phase 6b 只建模，不做运行时选择。
- `screen_anchor`、`transparency`、`blur`、`animation` 这些更细能力先留在设计文档和后续 PoC，
  不急着扩大共享 `CapabilityId`。

## 共享边界

共享层负责：

- `OverlayCmd` command queue。
- `OverlayModel` 状态和 TTL。
- layout 几何和文本截断。
- theme token 解析。
- 状态颜色、文本颜色、surface token。

Renderer 负责：

- 平台窗口创建。
- show/hide/fade/resize。
- material fallback。
- 文本绘制和图标/动画绘制。
- 锚定策略。

Renderer 不应直接读取业务配置；应只消费合并后的 effective overlay config。

## Phase 6a Renderer Facade

Phase 6a 先抽 renderer 选择边界，不改变 macOS renderer 行为：

- `overlay::renderer` 拥有平台 renderer backend 选择和 unsupported fallback。
- `overlay::run(rx, cfg)` 保持上层 API 不变，只转发到 `renderer::run(rx, cfg)`。
- macOS backend 继续调用 `overlay::macos::run()`；AppKit view/chrome/icon_fx、动画、窗口层级、
  focused window 锚定和 material fallback 不变。
- `command.rs`、`model.rs`、`layout.rs` 仍是共享层，不 import `overlay::macos` 或平台 SDK。
- Phase 6a 不实现 Windows/Linux renderer，只保留明确 unsupported fallback；Windows/Linux
  骨架和 PoC 留给后续阶段。

## Phase 6b Renderer Capability Skeleton

Phase 6b 只补 renderer capability/status skeleton，不改变任何绘制行为：

- `overlay::renderer` 同时拥有 backend 选择和 overlay-specific capability snapshot。
- `overlay::run(rx, cfg)` 的 macOS 分支仍直接调用 `overlay::macos::run(rx, cfg)`。
- 不修改 `OverlayCmd`、`OverlayModel`、layout、theme parser、AppKit view/chrome/icon_fx。
- 不实现 Windows/Linux renderer，也不把 capability snapshot 当成实时 permission probe。

## Phase 6c Doctor Capability Consumption

Phase 6c 只把 renderer capability snapshot 接入 `shuo doctor` 的现有 capability summary：

- doctor 先读取 `platform::capability::current_platform_capabilities()`，再用
  `overlay::renderer_capabilities()` 覆盖同 `CapabilityId` 的 overlay 条目。
- 这是只读诊断合并，不创建 overlay window，不启动 daemon，不读取业务配置。
- doctor 的错误/警告计数和退出码语义不变；capability summary 仍是非阻断信息。
- 不接入 TUI/GUI，不实现 Windows/Linux renderer。
