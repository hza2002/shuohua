# Cross-Platform Config And Theme

## 目标

用户应能同步同一套 `config.toml`、profile、ASR/post 配置和 theme 到 macOS、Windows、Linux。
平台差异通过 capability 诊断和 theme 平台段处理，不要求用户维护三份主配置。

## 主配置

`config.toml` 只放行为级通用配置：

- hotkey 语法和语义。
- voice/VAD/record audio/auto paste。
- profile routing。
- ASR/post 链接关系。
- UI language/theme 选择。
- overlay 行为：position、max_text_lines 等。

不把平台视觉实现细节放进主配置。

## Theme

Theme 分为共享 token 和平台覆盖：

```toml
[overlay.surface]
material = "liquid_glass"
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
material_backend = "liquid_glass"

[overlay.windows]
material_backend = "mica"

[overlay.linux]
material_backend = "blurred_glass"
```

当前平台读取通用字段和自身平台段；其他平台段忽略。未知字段仍按 schema 诊断，不静默吞掉
拼写错误。

## 降级规则

Theme 表达用户偏好，renderer 决定实际能力：

- 用户偏好 `liquid_glass`，平台支持则使用。
- 不支持时降级 `blurred_glass`。
- blur 不可用或可读性不足时降级 `translucent`。
- 仍不可读时降级 `solid`。

降级结果应进入 capability/status，供 doctor/TUI/GUI 显示。

## 字段治理

- 导出到官方模板的字段必须有运行时使用点或明确是 metadata。
- 实验字段可以保留 schema 支持，但不应默认出现在 starter config。
- 平台私有调试字段应放在平台段，并在文档里标记为 advanced。
- 删除无效字段时同步更新 schema、template、i18n 和用户迁移说明。

`theme.name` 是 metadata，不参与渲染，但可用于内置 theme registry 和 GUI 展示。
