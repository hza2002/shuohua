# config — 配置加载与热重载

**TL;DR**：`notify` 监听配置**目录**不监听文件（编辑器换 inode 会丢事件）；subscriber 自带 diff 只在关心字段变化时动作；`config.toml`/`theme/*.toml` 立即生效，profile/asr/post 下次录音才读。

> **何时读**：改配置 schema、热重载、profile 路由落地、theme。
> **不在这里**：profile 选哪套 ASR/post 的语义见 [post](post.md)；字段格式不是 schema（那是 history/UDS，见 [schema](../schema.md)）。
> **代码**：`src/config/`（`main.rs`/`profile.rs`/`theme.rs`/`schema.rs`/`spec.rs`/`diagnostics/`/`template/`/`asr/`/`post/`）；热重载在独立的 `src/reload.rs`。

## reload 模块边界

`reload.rs` 单向依赖 config/overlay/i18n/hotkey 的对外 API（`OverlayHandle`、`i18n::init`、`hotkey::parse`），**不被它们反向 import**——一个集中的"翻译层"：watcher 一个 source，subscriber N 个 sink。

- `watch_with_handle(path, overlay)` → notify watcher（专用 std::thread）+ 手动 reload handle
- `spawn_overlay` / `spawn_i18n` / `spawn_hotkey` 三个 subscriber，各自 diff `prev` 只对关心字段动作。
- `Rx = watch::Receiver<Arc<RuntimeConfig>>`（含主配置 + effective theme + fallback warning）。

## 实现要点

- **监听目录而非文件**：编辑器保存常 atomic rename 换 inode，监听文件本身丢事件。
- 自动 reload 只把 `config.toml` + `theme/*.toml` 当触发源；`profile/*.toml`/`asr/*.toml`/`post/**` 不触发 broadcast，下次录音开始同步读最新。
- **150ms debounce**（一次保存常触发 2-3 事件）。
- **parse 失败保留旧值**：只打日志 `config reload failed; keeping previous config`，不发空值。

## 字段覆盖矩阵

| 字段 | 生效 | 路径 |
|---|---|---|
| `[overlay].*` | 立即（next render） | `spawn_overlay` → rebuild_chrome |
| `ui.language` | 立即（重译 label） | `spawn_i18n` → `i18n::init` + `Relabel` |
| `[hotkey].trigger` | 立即（下次按键） | `spawn_hotkey` → mpsc<Combo> → 主循环换 Tracker+Suppressor |
| `[ui].theme*` / `theme/*.toml` | 立即 | spawn_overlay / TUI reload 重载 effective theme |
| `[voice].*` 全部 | 下次起 session | 主循环 `cfg_rx.borrow()` 取快照 |
| `[profile]` 路由 / `profile/*.toml` / `post/**` | 下次起 session | toggle ON 时选 Profile |
| 手动 `{"op":"reload_config"}` | 立即 | 走 UDS server，复用同一 parse+broadcast 入口 |

## voice preprocess

`[voice.preprocess].backend` 默认是 `apple`：使用 macOS 系统语音处理采集，把增益、回声和环境噪声处理放在输入链路里。`off` 是原始采集，不做预处理。模板必须导出真实默认值，并在注释里说明这两个已支持取值；未实现的后端不要写进用户模板说明。

## Hotkey trigger 热替换

CGEventTap 在 OS 层捕获所有键盘事件、不过滤——trigger 切换只影响 `Tracker.on_raw()` 判定。重置成本 = 主循环 select 收到新 keycode → `Tracker::new(new_code)`（归零 `trigger_pressed`，避免旧 trigger 半按串到新），不拆 CGEventTap。parse 失败保留旧 trigger。

## Theme

`theme/<id>.toml` 描述 TUI+overlay 颜色和少量 overlay token；字段缺省从内置 `gruvbox-dark` 补齐，用户文件优先于同名内置 preset。`theme_tui`/`theme_overlay` 可单独覆盖，空字符串跟随 `theme`。内置 theme 唯一来源是 `assets/themes/*.toml`，`build.rs` 编译期校验并生成嵌入 registry。
