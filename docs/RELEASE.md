# 发版手册

shuohua 的发版操作手册。**任何人或 agent 在执行发版动作前都必须完整读一遍本文档**。

> ⚠️ **给 Agent 的硬性要求**
>
> 发版是不可逆的（tag 推上去之后回滚成本很高）。本手册中标 **【需用户确认】** 的每一步，agent 都必须：
> 1. 把当前掌握的信息（版本号、commit 列表、CHANGELOG 草案、artifact 大小等）**完整展示**给用户
> 2. **明确请求确认**，例如"以上信息是否正确？要继续吗？"
> 3. **等待用户明确同意**后才进入下一步
>
> 用户没说"继续"/"yes"/"对"之前，不允许执行任何会改动 git / 文件系统 / 远端的命令（包括 `cargo release`、`git tag`、`git push`、`gh release` 等）。
>
> 跳过确认步骤即使最终结果正确也算违规。**慎重 > 效率**。

---

## 1. 概述

### 1.1 发版做什么

- 在本地用 `cargo-release` 工具：bump 版本号、commit、打 tag、push
- GitHub Actions 检测到 `v*` tag 后，自动在 macOS arm64 上构建 binary，打 tarball，上传到 GitHub Release 页面
- 用户从 Release 页面下载 `shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz`，解压安装

### 1.2 必须了解的限制（影响发版决策）

**未签名分发**（macOS specific）：项目当前未做 Apple Developer ID 签名 / 公证。这导致：

- 用户首次安装需要授权两项权限：**Microphone**、**Accessibility**
- **每次升级都要重新授权这两项**（macOS TCC 按 binary 内容 hash 识别未签名程序，版本一升 hash 就变，权限就掉）
- 唯一彻底解决方案是付费 Apple Developer 账号（$99/年）+ 签名 + 公证，本项目暂未启用

**对发版节奏的直接影响**：每次发版 = 用户多一次"重新授权"摩擦。所以**不为单 commit 发版**，攒到一组有意义改动再发。

### 1.3 平台与目标

- 当前唯一目标：`aarch64-apple-darwin`（Apple Silicon macOS）
- 不支持 x86_64 / Universal Binary
- Runner：GitHub Actions `macos-14`
- 跨平台（Linux / Windows）是中长期目标，代码已按 `_darwin.rs` 边界做了准备。跨平台扩展规划见 §6

---

## 2. 发版纪律

### 2.1 版本号规则（当前 0.x 阶段）

| 改动类型 | bump 方式 | 例子 |
|---|---|---|
| bugfix、refactor、文档、性能 | **patch** `0.1.0 → 0.1.1` | 修复某个 race condition |
| 新功能、新 ASR provider、配置 schema **加字段** | **minor** `0.1.0 → 0.2.0` | 新增豆包 v2 provider |
| 配置 schema **删/重命名字段**、history schema 升 version、CLI 子命令删改 | **minor**（必须加 ⚠️ Breaking 标注） | `[overlay.glass]` → `[overlay.macos]` |
| 进入 `1.0.0` | 只在愿意承诺配置 / IPC schema 进入稳定期时做 | 暂未到 |

**0.x 阶段所有 breaking change 都走 minor**，符合 Cargo / SemVer 对 0.x 的约定。升 1.0 是一个明确的承诺动作，不要顺手做。

### 2.2 发版节奏

- 不为单 commit 发版
- 攒到至少 3–5 个有意义改动、或 1 个用户可感知的功能再发
- 严重 bug fix 例外，但 CHANGELOG 顶部标 ⚠️ "推荐升级"

### 2.3 CHANGELOG 规则

- 写在 `CHANGELOG.md` **顶部**，新版本在最上面
- **完全人工撰写**。`cargo-release` 不会自动改 CHANGELOG（这是有意的：自动生成的 CHANGELOG 没有信息量）
- 模板见 §3.3

---

## 3. Agent 发版操作流程

### Step 0 — 前置环境检查 【需用户确认】

Agent 必须执行并展示以下信息：

```bash
git status --short --branch -uall
git branch --show-current
git log -1 --oneline
git fetch origin && git status -sb     # 看是否落后远端
git describe --tags --abbrev=0 2>/dev/null || echo "<no previous tag>"
```

向用户展示：

- 当前分支（**必须 `main`**）
- working tree 状态（**必须干净**）
- 最新 commit
- 是否与 origin/main 同步
- 上次 tag（用于下一步对比 commit 范围）

**确认问题**：环境是否符合发版前提？是否继续？

任何一项异常（不在 main / 有未提交改动 / 落后远端），**停下**让用户处理，不要自作主张 `git stash` / `git pull` / `git checkout`。

### Step 1 — 确定 bump 类型 【需用户确认】

Agent 展示自上次 tag 以来的全部 commit：

```bash
git log <last-tag>..HEAD --oneline
```

然后**逐条分析**每个 commit 属于 §2.1 表中的哪类，给出**推荐的 bump 类型 + 理由**，例如：

> 自 v0.1.0 以来有 4 个 commit：
> - `38d91f9 Fix state subscribe and Doubao close handling` → bugfix
> - `0149e3f fix: align theme diagnostics with macOS schema` → bugfix
> - `57fc23b fix: harden voice session finalization` → bugfix
> - `52379ed fix: restore cancel-first hotkey tracking` → bugfix
>
> 全是 bugfix，无 schema 变化。**推荐：patch（0.1.0 → 0.1.1）**

**确认问题**：推荐的 bump 类型对吗？或者你要指定具体版本号？

### Step 2 — schema 文档一致性自查 【需用户确认】

如果本期 commit 涉及下列任一改动，agent 必须读对应文档确认已同步：

| 改动 | 必须同步的文档 |
|---|---|
| 配置 TOML 字段 | `docs/CLI.md`、`docs/DESIGN.md` |
| history schema | `docs/SCHEMA.md`（删字段或破坏兼容必须升 schema version） |
| UDS 协议 | `docs/SCHEMA.md` |
| CLI 子命令 | `docs/CLI.md` |
| 模块边界 | `docs/MODULES.md` |

如果文档与代码不一致，**先停下补文档**，再回到 Step 1（commit 历史变了，bump 判断可能要重做）。

**确认问题**：所有相关文档是否已与代码同步？

### Step 3 — 起草 CHANGELOG 【需用户确认】

Agent 按以下模板起草，**展示完整文本**给用户：

```markdown
## v0.1.1 - YYYY-MM-DD

⚠️ Breaking: [仅 breaking 版本出现此行；写清楚破坏点 + 迁移方法]

### Added
- 新功能 A

### Fixed
- 修复 X 在 Y 场景下崩溃（commit hash）

### Changed
- 调整 Z 默认值（commit hash）

[可选行] 本版要求重新授权 Microphone + Accessibility。
```

**确认问题**：CHANGELOG 草案是否准确、是否遗漏？日期对吗？要不要加"重新授权"提醒（如果用户感知度高就加）？

用户确认后，agent **写入** `CHANGELOG.md` 顶部（在最旧版本的上面，在文件最顶 H1 标题的下面）。写入后立刻：

```bash
git add CHANGELOG.md
```

**不要自己 commit**——`cargo release` 在 bump 阶段会把 Cargo.toml 改动和这个已 stage 的 CHANGELOG 一起打到 release commit 里。**如果忘了 `git add`，cargo-release 会因 working tree 不干净而 abort，或者 release commit 不含 CHANGELOG。**

### Step 4 — 手动冒烟测试 【需用户确认】

Agent 提醒用户在本地至少手动跑过：

- `cargo build --release` 成功
- `./target/release/shuo doctor` 输出正常
- 至少 1 次 voice 输入端到端（按下热键 → 说话 → 上屏）
- TUI 启动正常（`./target/release/shuo`）

**Agent 不能替用户完成这步**（涉及录音权限、麦克风、上屏，必须真人验证）。

**确认问题**：你已经手动验证过核心路径正常？

### Step 5 — Dry run `cargo release` 【需用户确认】

```bash
cargo release <patch|minor|<version>> --dry-run
```

Agent 展示 dry-run 输出，重点确认：

- 新版本号（`<old> → <new>`）
- commit message 是 `release: vX.Y.Z`
- tag 名是 `vX.Y.Z`
- 将要 push 到 `origin main` 和 `origin vX.Y.Z`
- pre-release-hook 已通过（fmt / clippy / test）

**确认问题**：以上操作清单是否符合预期？要继续真正执行吗？

### Step 6 — 真正执行 `cargo release` 【需用户确认】

⚠️ **这是不可逆动作的最后一道关卡**。Agent 在执行前必须再次明确请求用户确认：

> 即将执行 `cargo release <type> --execute`，这会：
> 1. 修改 `Cargo.toml` 版本号
> 2. 跟 `CHANGELOG.md` 一起 commit（message: `release: vX.Y.Z`）
> 3. 打 tag `vX.Y.Z`
> 4. push commit 和 tag 到 `origin`
>
> 推上去之后 GitHub Actions 会立刻开始构建。**确认执行吗？**

用户明确同意后才能跑：

```bash
cargo release <patch|minor|<version>> --execute
```

如果中途任何步骤失败（pre-release-hook 失败、push 失败等），**停下报告状态**，让用户决定怎么办。**禁止用 `--no-verify` 跳过 hook**，禁止自动重试。

### Step 7 — 监控 Actions 构建 【需用户确认】

Agent 用 `gh` 命令监控：

```bash
gh run list --workflow=release.yml --limit=1
gh run watch <run-id>
```

构建完成后展示：

- 构建状态（success / failure）
- 总耗时
- 任何 warning 或异常

**确认问题**：构建结果正常吗？

如果失败，agent **只报告失败原因 + 日志关键行**，不自动修复，不自动 re-run。失败处理见 §4。

### Step 8 — 验证 Release artifact 【需用户确认】

构建成功后，agent 展示：

```bash
gh release view vX.Y.Z
```

确认：

- **artifact 名称正确**：`shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz` + 同名 `.sha256`
- **artifact 大小合理**：首次发版时记录实际大小，更新此处作为基线。后续发版偏离基线 ±50% 必须由 agent 显式指出（可能是 build 出错、漏 strip、误打包多余文件）。Rust release binary 一般不会突然胀缩
- **sha256 文件存在且非空**
- **Release notes 内容正确**（默认从 commit 生成，也可能需要手动补 CHANGELOG 内容）

下载 tarball 本地校验：

```bash
gh release download vX.Y.Z -p '*.tar.gz' -p '*.sha256' -D /tmp/shuo-release-check
cd /tmp/shuo-release-check
expected=$(cat *.sha256 | tr -d '[:space:]')
actual=$(shasum -a 256 *.tar.gz | awk '{print $1}')
[ "$expected" = "$actual" ] && echo "sha256 OK" || echo "sha256 MISMATCH: $expected vs $actual"
tar -tzf *.tar.gz       # 看内容结构
```

预期 tarball 内容结构：

```
shuo-vX.Y.Z-aarch64-apple-darwin/
├── shuo            (binary)
├── LICENSE
└── README.md
```

**确认问题**：artifact 大小、sha256、内容结构是否都符合预期？

### Step 9 — 真实试装 【需用户确认】

Agent 提示用户在自己机器上下载、解压、跑：

```bash
xattr -d com.apple.quarantine ./shuo 2>/dev/null   # 绕 Gatekeeper（首次）
./shuo --version       # 确认版本号显示正确
./shuo doctor          # 确认权限检测正常
```

**Agent 不能替用户完成这步**（涉及 macOS 权限弹窗、Gatekeeper 提示，必须真人操作）。

**确认问题**：版本号显示正确？doctor 输出正常？升级后权限是否需要重新授权（如预期）？

### Step 10 — 发版完成

Agent 报告本次发版摘要：

- 版本号
- artifact 大小
- Release 页面链接
- 主要改动一句话总结
- 任何需要后续跟进的事项（如：试装时发现的小问题、待写的 GitHub issue 等）

---

## 4. 出问题怎么办

### 4.1 `cargo release` pre-release-hook fail（fmt / clippy / test 不过）

- cargo-release 自动停下，没有产生 commit 也没有 push
- 修代码 → 重新 commit → 回到 Step 5 重跑

**不要**用 `--no-verify` 或 `--skip-tag` 跳过。这些 hook 是设计上的安全网。

### 4.2 CI fail 在 tag 一致性校验

`Cargo.toml` 版本与 tag 对不上。理论上 `cargo-release` 保证一致，出现说明有人手动打了 tag。

- 删 tag（本地 + 远端）：
  ```bash
  git tag -d vX.Y.Z
  git push origin :refs/tags/vX.Y.Z
  ```
- 回到 Step 1 重新走流程

### 4.3 CI fail 在 fmt / clippy / test

本地能过 CI 不过，最常见原因是本地 Rust 工具链版本与 macos-14 runner 上的 stable 不同。

- 在 main 上直接 fix（不需要新 tag），push commit
- 不需要重发版：tag 仍指向旧 commit，手动 re-run：
  ```bash
  gh run rerun <run-id> --failed
  ```
- 如果 fix 涉及代码改动（不只是 CI 配置），新建一个 patch 版本重发

### 4.4 tag 已 push 但发现版本号或 CHANGELOG 写错了

- 删 tag（本地 + 远端，命令见 4.2）
- 改 `Cargo.toml` / `CHANGELOG.md` 回到正确状态，commit
- 回到 Step 1

**不要**"补一个修正版"——版本号在历史里跳号会很难看。

### 4.5 Release 已对外发出但代码有严重 bug

- GitHub 上把那个 Release 标记为 "Pre-release" 或直接 Delete（已下载的用户不会回滚，但能阻止后续用户拿到坏版本）
- 立刻发一个 patch 版（如 v0.1.1 → v0.1.2）含修复 + CHANGELOG ⚠️ 推荐升级标注
- 在 README / 项目主页加临时提示（如果有外部用户）

### 4.6 Artifact 大小异常

预期 8–20 MB。如果显著偏离：

- 偏小：可能 release 模式没生效、binary 没正确包含进 tarball
- 偏大：可能 debug 符号没 strip、误把 `target/` 或日志打进 tarball
- 任一情况都**不要发出去**，删 tag、修 workflow、重发
- 首次发版后回来更新 §3 Step 8 的"基线大小"

---

## 5. 首次发版的特殊准备

只在第一次正式 release 之前做一次。

### 5.1 已完成的基础设施（基线）

下列由 `2026-06-20-github-release-packaging` 计划完成，已 commit 在 main：

- `LICENSE`（MIT 文本，匹配 Cargo.toml 声明）
- `README.md`（项目简介 + 安装 + 权限 + 链到 docs/）
- `release.toml`（cargo-release 配置：限制 main 分支、强制 pre-release-hook、禁止 publish）
- `.github/workflows/release.yml`（tag-driven macOS release workflow）
- `.github/release-body.md`（Release notes 模板含权限提醒）
- `docs/RELEASE.md`（本文档）
- 本地 `cargo install cargo-release` 已装

### 5.2 还需要做的加固（首次公开发版前）

[docs/RELEASE_HARDENING.md](RELEASE_HARDENING.md) 列出 7 个必须按顺序走完的步骤：

1. 配置 git remote 并首次 push 到 GitHub
2. 撤销本仓库 `commit.gpgsign` override
3. 装 `includeIf onbranch:main` 让 main 自动 GPG 签名
4. 改 `release.toml` 让 release commit + v\* tag 强制签名
5. 更新 `CLAUDE.md` 加 agent 工作流硬规则
6. GitHub branch protection（main 要求签名）
7. GitHub tag protection（v\* 限制 pusher）

走完 7 步后再回到本文档执行真实发版。

### 5.3 决定首个 tag 版本号

- 保持与 Cargo.toml 一致：`v0.1.0`
- 或首版前还有 fix 进入：先 patch 几次再发，或直接首个 tag 用 `v0.1.1` 等

---

## 6. 未来扩展（暂未启用）

下列项启用时需要回头更新本文档对应章节。

### 6.1 Apple Developer ID 签名 + 公证（macOS）

消除每次升级重新授权的摩擦。需要付费 Apple Developer 账号（$99/年）。启用后本文档 §1.2 整段权限说明需要重写。触发条件：

- 自己日常使用被升级重授权烦到
- 出现 ≥5 个稳定外部用户反馈此问题

### 6.2 Homebrew tap（macOS）

让用户 `brew install shuo` 一键升级。在 release workflow 末尾加自动 bump formula 的 job，artifact 命名 / sha256 / tarball 结构已经按 brew 友好预留。

**与 §6.1 签名强相关**——不签名就上 brew 会让 brew upgrade 用户体验更差（每次升级都要在弹窗里点授权），所以两者通常一起启用。

### 6.3 跨平台扩展（Linux / Windows）

代码层面已按 `_darwin.rs` / `overlay/macos/` 边界做了准备。release 层面需要：

- 把单 job workflow 改成 matrix（`[macos-14, ubuntu-latest, windows-latest]`），每平台独立 build
- artifact 命名规则已是 Rust target triple 形式（`shuo-vX.Y.Z-<triple>.<ext>`），新平台是平行扩展，**不需要改命名**：
  - `shuo-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`
  - `shuo-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz`
  - `shuo-vX.Y.Z-x86_64-pc-windows-msvc.zip`（Windows 用 `.zip`）
- `Cargo.toml` 加 `[target.'cfg(target_os = "macos")'.dependencies]` 分组
- 本文档 §1.2 权限说明改成分平台子章节：macOS TCC / Linux PipeWire+Wayland / Windows 低层键盘 hook 提权
- 签名策略每平台独立：Apple Developer ID / Windows EV cert / Linux GPG
- 分发渠道每平台独立：Homebrew tap (mac) / AUR + deb + AppImage (linux) / Scoop + winget (win)

**重新评估 `cargo-dist` 的触发点**：当第一个非 macOS 平台真的能 build pass 时。手写 workflow 在 1 平台够清晰，3 平台 × 多渠道开始失控。届时切 cargo-dist 是合理的，迁移成本主要在 workflow 重写，命名约定 / 版本规则 / CHANGELOG / 操作流程都保留。
