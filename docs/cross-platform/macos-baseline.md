# macOS Cross-Platform Baseline

Phase 0 的目标是记录跨平台重构前的 macOS 回归基线。本文档只记录可验证的不变量、
自动门禁和需要用户在真实 macOS 会话里手动确认的 checklist；它不表示这些手动项已由
agent 验证。

## 自动验证基线

提交跨平台阶段前运行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

这些命令覆盖格式、静态检查、纯函数状态机、配置/schema/i18n、IPC protocol、
history/TUI 纯逻辑、voice fake recorder 生命周期和平台边界架构测试。

可按需先跑更小集合定位问题：

- `cargo test hotkey`：hotkey grammar、tracker、suppress down/up 配对。
- `cargo test voice`：stop、cancel、finalize、error/timeout 路径。
- `cargo test tui`：Status、History、Configure 页面状态逻辑。
- `cargo test platform_layout`：平台目录和 macOS-only import 边界。

`shuo doctor` 可作为本机诊断检查，但不替代自动门禁；`--runtime` 会触发真实 provider
检查，除非本阶段明确需要，否则不纳入默认基线。

## 平台边界审计

当前允许保留的 macOS-only 边界：

- `src/platform/macos/**`：AppKit、CoreGraphics、AX、clipboard、permissions 等 macOS
  backend。
- `src/platform/{autotype,clipboard,permissions,daemon}.rs`：共享 facade，按
  `cfg(target_os = "macos")` 转发到 macOS backend，非 macOS 返回明确 unsupported 或
  conservative default。
- `src/hotkey/provider_darwin.rs` 和 `src/hotkey/mod.rs`：CGEventTap provider，Phase 5
  再抽 desktop capability/backend。
- `src/overlay/macos/**`：AppKit renderer，Phase 6 再抽 renderer boundary。
- `src/cli/service/macos.rs` 和 `src/cli/service/mod.rs`：launchd service backend，Phase 4
  再抽 service manager。
- `src/cli/app/platform/**`：self-update 的平台 adapter。

已知后续阶段要处理但 Phase 0 不抽的边界：

- `src/ipc/{client,server}.rs` 和 `src/daemon/{lock,fallback}.rs` 仍使用 UDS / Unix socket /
  Unix process primitives；这是 Phase 3/4 范围。
- `src/cli/doctor.rs` 仍有 launchd-centric 诊断文案和 service status；Phase 1/4 后应通过
  capability/service manager status 表达。
- `src/post/app_context.rs` 当前作为 post 层平台入口直接转发到 macOS app context；后续
  desktop capability boundary 可以收敛到更统一的 facade。

## 手动验证 Checklist

这些项目依赖 macOS 权限、真实前台 App、系统 UI、麦克风、launchd 或 AppKit，不能由
CI/单测稳定替代。未由用户在真实环境执行并记录前，不应声称手动基线已完成。

### Hotkey

- 默认 `right_option:double` 能启动录音，前台 App 不出现可见输入副作用。
- 再次触发 toggle 能进入 Stop 收尾，不会卡在录音中。
- `escape` cancel 能中断当前 session，并关闭 lingering overlay。
- 修改 `[hotkey].trigger` 后保存 `config.toml`，无需重启 daemon，下次按键按新 trigger
  生效。
- keyed trigger 被 suppress 时，前台 App 不出现孤立 keyup、modifier 卡住或快捷键泄漏。

### Stop

- 录音中说一句话后 stop，尾字不被截断，说明 residual PCM + `stop_delay_ms` 生效。
- stop 后进入识别收尾路径，最终文本进入剪贴板或上屏。
- stop 后 post、dispatch、history 正常执行，TUI 能看到完成记录。
- VAD pause 配置下，静音 idle 状态触发 stop 也能完成，不需要靠 ESC 退出。

### Cancel

- 刚启动录音后立即 cancel，若没有语音内容，不产生 history 记录或 retained audio。
- 已说话后 cancel 停止上屏/dispatch；如已有可归档内容，生成 `status=canceled` history。
- post-processing 期间 cancel，不继续写剪贴板或 auto paste。
- cancel 后下一次录音正常开始，不继承旧 partial、segments 或 overlay error。

### Overlay

- 启动录音时 overlay 出现并锚定当前 focused window，状态正常变化。
- 录音中电平条或活动状态随输入变化，静音/说话有可见差异。
- ASR partial/segment 在 overlay 文本区更新，长文本截断/换行不遮挡 UI。
- post warning 使用 Notice 并延迟 hide；Error 保留足够时间，ESC 可 dismiss。
- 修改 overlay/theme 配置后保存，下一次 render/reload 体现变化，不需要重启 daemon。

### Clipboard/Paste

- `voice.auto_paste = false` 时，完成录音只写剪贴板，手动 paste 可粘贴最终文本。
- `voice.auto_paste = true` 时，在 TextEdit、Notes、浏览器输入框等真实前台 App 中能自动
  粘贴。
- Accessibility 权限不可用或目标 App 拒绝注入时，文本仍留在剪贴板。
- 空识别、terminal error、ASR finalize timeout 不污染剪贴板或触发 paste。
- post processor 修改后的最终文本是剪贴板/上屏文本，而不是原始 ASR 文本。

### TUI

- daemon 运行时执行 `shuo` 能进入 TUI 并显示 Status；关闭 TUI 不影响 daemon。
- Status 页实时显示 idle/recording/stopping、duration、words、segments/partial 和
  session meta。
- History 页能加载最近记录、搜索、分页，并看到刚完成的 `history_appended` 记录。
- Configure/Settings 页展示配置/诊断状态；手动 reload config 后 UI 状态更新。
- Tab、Shift-Tab、`1/2/3`、`/`、`q` 等导航行为正常。

### Service Lifecycle

- `shuo service install` 后 launchd plist 存在，daemon 能启动，`shuo service status`
  显示 running、pid、uptime、state。
- `shuo service stop` 通过 UDS graceful shutdown，daemon 退出后不被 KeepAlive 立即重启。
- 录音中执行 stop/restart 时，daemon 尝试 bounded graceful shutdown，不强杀或留下坏状态。
- `shuo service restart` 先确认旧 PID 退出，再启动新 daemon；status 显示新 PID。
- `shuo service uninstall` 移除 plist，不删除 binary、config、history 或用户数据。

### History

- 成功提交后，`~/.local/state/shuohua/history/YYYY-MM.jsonl` 追加一条记录，字段包含
  status、text、asr、pipeline、app、timestamps。
- 早期失败、无内容 cancel 不写 history；有内容 cancel 写 `status=canceled`。
- terminal error/finalize timeout 写失败状态时，不执行 post/clipboard/paste。
- History 页删除 audio 只删 retained audio，不删 JSONL；删除 history 同时尝试删同 ID
  audio。
- JSONL 保持 UTF-8、单行 JSON、月分片；TUI stats/analytics 能反映新增记录。
