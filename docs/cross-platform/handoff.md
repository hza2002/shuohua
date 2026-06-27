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
  `IDCompositionVisual3::SetOpacity2` panel opacity binding。DirectComposition device 现在来自 D3D11 BGRA
  `IDXGIDevice`，hardware D3D11 不可用时 fallback 到 WARP，避免 NULL-device `CreateSurface` 路线触发
  `0x8000000E`。Surface 创建已延后到第一次 scene update：
  startup 只创建 device/target/visual tree，避免 daemon 初始化时过早调用 `IDCompositionDevice::CreateSurface`
  触发 `0x8000000E`。Composition probe 现在也对齐 Direct2D fallback
  的 shadow outset geometry：surface 包含 renderer-owned outset，panel/content 坐标保持 inset。但还未绘制
  最终 material/shadow/animation，也未切换默认 backend。当前 composition shadow surface 只验证独立
  `shadow` visual 分层和 tapered shadow pass plumbing，不代表最终阴影质感。Icon glyph 已从 panel surface
  拆到独立 icon surface，`icon` visual 会按状态 icon plan 切换 looping state-driven opacity animation profile；
  这只证明独立 icon surface、animation binding 和状态路由可用，不代表 transform/scale/rotate 状态 icon 动画完成。
  Composition text/icon surface 使用 Direct2D 默认 text antialiasing，不再强制 grayscale。
  `SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_VISIBLE` 不能再让当前 `WS_EX_LAYERED` host 接管可见输出：
  手动 QA 发现同一个 HWND 同时作为 `UpdateLayeredWindow` layered window 和 DirectComposition target
  会出现启动后只剩边缘残影/持续刷新感。当前代码保留 `SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_PROBE`
  作为旁路初始化和 surface draw 探针，但可见输出强制回到 Direct2D per-pixel fallback；如果 probe
  scene update 失败，会禁用 probe，避免每帧重复 warning。下一步需要单独设计真正的
  composition-backed host/window，而不是在现有 layered-window host 上继续打开 visible gate。
- Cross-user 第二账号隔离验证延后；代码已有 user/session scoped pipe/mutex 方向，但不同用户实机
  smoke 未完成。
- Windows release-grade 验收仍缺 multi-monitor、remote desktop/UAC/elevation、更多目标应用、
  长时间录音 soak、多设备/权限矩阵。
- Linux runtime backend 尚未接完整链路。

## 验证结果

最近已知 Windows core 验证通过范围见当前分支提交历史；清理 GUI PoC 后需要重新跑：

- `cargo fmt --check`
- `cargo test --test doc_consistency`
- `cargo test --test platform_layout`
- `cargo test --target x86_64-pc-windows-msvc`
- `cargo build --target x86_64-pc-windows-msvc`
- 必要时补 `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings`
- `SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_PROBE=1 SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_VISIBLE=1 cargo test --target x86_64-pc-windows-msvc
  overlay::windows::tests::runtime_smoke_creates_shows_hides_and_quits_window -- --ignored --nocapture`
  已通过，可验证 DirectComposition visible probe 初始化、panel/icon surface draw、looping icon opacity animation
  binding 和 DirectWrite text draw 不破坏 Direct2D fallback。

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
