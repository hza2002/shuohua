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

### Linux

Wayland-first。X11 只保留 backend 接口位置，成本过高时允许 unsupported。

Wayland renderer 目标：

- 优先 compositor 支持的 overlay/layer-shell 类能力。
- 支持不了置顶/穿透/精确锚定时，降级到普通半透明/solid 状态窗。
- 核心录音、文本、状态、错误提示必须可用。

X11 backend 不作为第一阶段目标。

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
