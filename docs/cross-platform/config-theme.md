# Cross-Platform Config And Theme

## 目标

用户应能同步同一套 `config.toml`、profile、ASR/post 配置和 theme 到 macOS、Windows、Linux。
平台差异通过 capability 诊断和 theme 平台段处理，不要求用户维护三份主配置。

配置和 theme 属于共享 product data。CLI、daemon、TUI、GUI 和 packaged desktop app 默认读取同一套
product config root；package/app 私有目录只能保存 GUI/runtime 私有状态，不能保存另一份有效配置。
路径归属规则见 [app-data.md](app-data.md)。

## 主配置

`config.toml` 只放行为级通用配置：

- hotkey 语法和语义。
- voice/VAD/record audio/auto paste。
- profile routing。
- ASR/post 链接关系。
- UI language/theme 选择。
- overlay 行为和用户偏好：position、max_text_lines、width、text_scale 等。

不把平台视觉实现细节放进主配置。

Overlay 尺寸和文字大小属于用户偏好，不属于 theme。用户切换 theme 时，颜色和材质可以变化，但
浮层宽度、最多行数和文字大小应保持稳定。对应字段是 `overlay.width`、`overlay.text_scale` 和
已有的 `overlay.max_text_lines`：

```toml
[overlay]
position = "bottom"
max_text_lines = 5
width = 572.0
text_scale = 1.0
```

Renderer 使用 `width` 和 `text_scale` 推导每行容量、行高和实际高度；不暴露 `height` 或
`chars_per_line`，避免这些字段与 `max_text_lines` / 字体渲染产生冲突。

Profile 内容三端共享，route matcher 按平台分段。`profile.routes` 使用 profile 名作为动态 key，
每个 profile 下只读取当前平台段；同一 app identity 命中多个 profile 是配置错误：

```toml
[profile]
default = "default"

[profile.routes.agent.macos]
bundle_id = ["com.microsoft.VSCode"]

[profile.routes.agent.windows]
app_user_model_id = ["Microsoft.VisualStudioCode"]
exe_name = ["Code.exe"]

[profile.routes.agent.linux]
desktop_id = ["code.desktop"]
wm_class = ["Code"]
process_name = ["code"]
```

不要把不同平台的应用 ID 混在同一个数组里。Windows/Linux backend 未能提供 active app identity 前，
这些平台会自然落到 `profile.default`。

`[dev]` 属于本机调试开关，不是跨平台同步契约。schema 继续接受这类字段以保持兼容，
但 starter config 不默认输出实验字段；需要时由开发者手动加入。

## Theme

Theme 分为共享 token 和平台覆盖。共享 token 是跨端契约，平台段只描述该平台 renderer
的偏好或调试开关：

```toml
[overlay.surface]
background = "bg"
background_alpha = 0.70
corner_radius = 18.0

[overlay.text]
primary = "fg0"
secondary = "fg1"

[overlay.state]
recording = "red"
thinking = "blue"

[overlay.macos]
glass_variant = 11
glass_style = "clear"

[overlay.windows]
material = "mica"

[overlay.linux]
material = "blurred_glass"
```

当前平台读取通用字段和自身平台段；其他平台段忽略。未知字段默认按 schema 诊断，不静默吞掉
拼写错误。

Phase 2 只让 schema 和 parser 接受 `overlay.windows.material`、`overlay.linux.material`
这类 future 平台偏好，macOS 运行时仍只消费通用 token 和 `overlay.macos`。平台段内字段
必须显式列入 schema；不能为了“可扩展”把整段设为 free table。

`overlay.surface.material` 暂不落 schema。material 降级属于 renderer/capability 结果，
不应在主配置或共享 surface token 中提前绑定具体平台效果。新增通用 material preference
前，需要先完成 overlay renderer boundary 设计评审。

## 降级规则

Theme 表达用户偏好，renderer 决定实际能力：

- 用户偏好 `liquid_glass`，平台支持则使用。
- 不支持时降级 `blurred_glass`。
- blur 不可用或可读性不足时降级 `translucent`。
- 仍不可读时降级 `solid`。

降级结果应进入 capability/status，供 doctor/TUI/GUI 显示。

## 字段治理

- 导出到官方模板的字段应有运行时使用点或明确是 metadata。
- 实验字段可以保留 schema 支持，但不应默认出现在 starter config。
- 平台私有调试字段应放在平台段，并在文档里标记为 advanced。
- 删除无效字段时同步更新 schema、template、i18n 和用户迁移说明。

`theme.name` 是 metadata，不参与渲染，但可用于内置 theme registry 和 GUI 展示。

当前 advanced 字段：

- `[overlay.macos].glass_variant`
- `[overlay.macos].glass_style`
- `[overlay.macos].subdued`
- `[overlay.macos].background_blur_radius`
- `[overlay.windows].material`
- `[overlay.linux].material`
