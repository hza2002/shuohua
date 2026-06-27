# Cross-Platform Handoff

## 当前分支

`feat/cross-platform-design`

## 最近 commit

以 `git log -1` 为准。GUI PoC 归档点是 `feat/gui-poc-archive`，指向清理前的
`5074ca4 docs: record audio processing research`。

## 当前 phase

Windows-first core runtime 收尾；GUI/Tauri PoC 已从当前 runtime 分支移出。

## 已完成事项

- 当前分支删除 GUI PoC 相关源码和静态资源：`src-tauri/**`、`gui-dist/**`、
  `src/client_api.rs`、`src/lib.rs`、`docs/cross-platform/gui.md`。
- TUI 已重新直接使用 `ipc::client::IpcClient` 和既有 `Command::Subscribe`，不依赖
  GUI client helper。
- `docs/cross-platform/audio-processing.md` 已记录后续 audio preprocessing 调研：
  WebRTC APM、Sonora、RNNoise、SpeexDSP、DeepFilterNet、backend 边界和单二进制风险。
- Windows core runtime 主链路已完成一轮 smoke/用户验证：hotkey、audio capture、Silero/VadPause、
  ASR、post、clipboard/paste、history、retained audio、path open/reveal、active app/profile route、
  IPC/service/single instance/process probe。
- Linux 仍保留 compile/capability/service dry-run 基线，方便后续接 runtime backend。

## 未完成事项

- Windows overlay 视觉与实现路线仍需重构；当前 Win32/GDI baseline 可用但不是最终质量。
- Cross-user 第二账号隔离验证延后；代码已有 user/session scoped pipe/mutex 方向，但不同用户实机
  smoke 未完成。
- Windows release-grade 验收仍缺 multi-monitor、remote desktop/UAC/elevation、更多目标应用、
  长时间录音 soak、多设备/权限矩阵。
- Linux runtime backend 尚未接完整链路。
- Audio preprocessing 暂作为后续独立能力，不在当前 Windows closeout 中继续扩展。

## 验证结果

最近已知 Windows core 验证通过范围见当前分支提交历史；清理 GUI PoC 后需要重新跑：

- `cargo fmt --check`
- `cargo test --test doc_consistency`
- `cargo test --test platform_layout`
- `cargo test --target x86_64-pc-windows-msvc`
- `cargo build --target x86_64-pc-windows-msvc`
- 必要时补 `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings`

## 已知风险

- 当前分支历史里曾经包含 GUI PoC；如果最终使用 squash merge，主分支只会看到清理后的结果。
- `feat/gui-poc-archive` 必须保留到未来 GUI 产品阶段重新评估完成。
- Windows overlay 的最终技术路线需要重新调研官方 Microsoft 文档，不应继续在当前 GDI baseline 上
  追加大量视觉补丁。

## 下一步建议

1. 完成 GUI PoC 清理验证并提交 `chore: archive gui poc off runtime branch`。
2. 调研现代 Windows overlay/window 技术路线，优先评估 DirectComposition/Direct2D/DirectWrite、
   Windows Composition、DWM backdrop 和 Windows App SDK 相关限制。
3. 选定 overlay 路线后先更新 `docs/cross-platform/overlay.md` 和 `docs/cross-platform/windows.md`，
   再小步实现。
