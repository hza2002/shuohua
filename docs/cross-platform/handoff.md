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

- Windows overlay 视觉与实现路线仍需重构；当前 Win32 + Direct2D/DirectWrite +
  `UpdateLayeredWindow` baseline 可用但不是最终质量。Composition backend infrastructure 已建立，可用
  `SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_PROBE` 探测 DirectComposition 初始化，但默认仍走 Direct2D
  fallback。Windows overlay scene 计划对象已抽出，Direct2D fallback 和后续 Composition renderer 应共享
  同一份状态/icon/meta/body 文本计划与 layout frames；Composition probe 已验证 root animation 创建/绑定/
  commit 路径，以及 panel `IDCompositionSurface` 创建/绑定、resize、`BeginDraw::<IDXGISurface>`、
  Direct2D `CreateDxgiSurfaceRenderTarget` 绘制圆角半透明 panel、DirectWrite 绘制系统 icon glyph 与
  state/stats/meta/body 文本、`EndDraw` 路径，并验证 `IDCompositionRectangleClip` rounded clip 与
  `IDCompositionVisual3::SetOpacity2` panel opacity binding，但还未绘制最终 material/shadow/animation，也未
  切换默认 backend。
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
- `SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_PROBE=1 cargo test --target x86_64-pc-windows-msvc
  overlay::windows::tests::runtime_smoke_creates_shows_hides_and_quits_window -- --ignored --nocapture`
  已通过，可验证 DirectComposition probe 初始化、panel surface draw 和 DirectWrite text draw 不破坏
  Direct2D fallback。

## 已知风险

- 当前分支历史里曾经包含 GUI PoC；如果最终使用 squash merge，主分支只会看到清理后的结果。
- `feat/gui-poc-archive` 必须保留到未来 GUI 产品阶段重新评估完成。
- Windows overlay 的最终技术路线需要重新调研官方 Microsoft 文档，不应继续在当前 GDI baseline 上
  追加大量视觉补丁。

## 下一步建议

1. Windows overlay 下一步先做 composition-backed renderer PoC 设计：保留 Win32 no-activate/topmost
   shell，评估 DirectComposition 或 Windows Composition 负责材质、圆角裁剪、阴影、opacity 动画；
   DirectWrite/Direct2D 负责清晰文本。
2. PoC 前不要继续在 GDI/DIB fallback 上堆视觉补丁，也不要直接接 Windows App SDK
   Mica/Acrylic 或 undocumented blur API。
3. 下一步填 `composition.rs` 最小 Windows-only renderer：先补 panel opacity/scale 动画和
   compositor-owned shadow/rounded clipping，再考虑 blur/material 与默认 backend 切换。
