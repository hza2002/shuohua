# Release

shuohua 的发版操作手册。发版会创建并推送 `v*` tag，触发 GitHub Actions
构建和发布 artifact。任何不可逆动作前都必须先向用户展示当前信息并等待明确确认。
除非步骤另有说明，以下命令都从仓库根目录执行。

## 1. 发版模型

- 本地使用 `cargo-release` bump 版本、创建 release commit、打 tag、push。
- GitHub Actions 在 `v*` tag push 后构建 `aarch64-apple-darwin` binary。
- Release 页面上传：
  - `shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz`
  - `shuo-vX.Y.Z-aarch64-apple-darwin.tar.gz.sha256`
- 当前 release 未做 Apple Developer ID 签名 / 公证。首次安装和每次升级后，用户通常需要重新授权 Microphone 和 Accessibility。

## 2. 版本规则

当前处于 `0.x`：

| 改动类型 | bump |
|---|---|
| bugfix、文档、重构、性能 | patch |
| 新功能、新 provider、配置 schema 加字段 | minor |
| 配置 schema 删/重命名字段、history schema 升 version、CLI 破坏性变更 | minor，并在 changelog 标 `Breaking` |

不要为单个普通 commit 发版。严重 bug fix 可以例外。

## 3. 发版前检查

只读检查：

```bash
git status --short --branch -uall
git branch --show-current
git log -1 --oneline --show-signature
git fetch origin
git status -sb
git describe --tags --abbrev=0 2>/dev/null || echo "<no previous tag>"
grep -E 'sign-commit|sign-tag' release.toml
```

必须满足：

- 当前分支是 `main`。
- working tree 干净，且与 `origin/main` 同步。
- 最新 commit 是 GPG signed / verified。
- `release.toml` 中 `sign-commit = true` 且 `sign-tag = true`。
- GitHub main ruleset 仍要求 signed commits，并禁止删除和 force push。
- GitHub `v*` tag ruleset 如已启用，不会阻止本次创建 release tag。

异常时停下，不自动 stash、pull、rebase、push 或改 ruleset。

## 4. 确定版本

查看自上个 tag 以来的提交：

```bash
last_tag=$(git describe --tags --abbrev=0 2>/dev/null || true)
if [ -n "$last_tag" ]; then
  git log "$last_tag"..HEAD --oneline
else
  git log --oneline
fi
```

按版本规则推荐 `patch`、`minor` 或具体版本号，并向用户确认。首个公开版本通常是 `0.1.0`。

## 5. 起草 CHANGELOG

发版时由 agent 根据自上个 tag 以来的 commit 起草本次版本段落，展示给用户确认后写入 `CHANGELOG.md` 顶部。不要在日常开发中维护 `Unreleased` 段。

格式：

```markdown
## vX.Y.Z - YYYY-MM-DD

### Breaking
- ...

### Added
- ...

### Changed
- ...

### Fixed
- ...

### Security
- ...
```

只保留非空 section。`Breaking` 仅在有迁移成本时出现，并放在最前面；`Security` 仅用于安全相关修复。避免 `Notes` 这类宽泛 section，平台要求、安装限制、签名状态等长期说明应写在 README 或 `.github/release-body.md`，不要每版复制进 changelog。

条目用用户视角写，避免内部过程描述，不逐条镜像 commit。GitHub Release 页面会额外生成 "What's Changed" 提交 / PR / contributor 列表。

确认后只 stage changelog，不手动创建 release commit：

```bash
git add CHANGELOG.md
```

`cargo release` 会把 `Cargo.toml` 版本变更和 staged changelog 一起放进 `release: vX.Y.Z` commit。

## 6. 手动冒烟

用户在本机确认：

```bash
cargo build --release
./target/release/shuo doctor
./target/release/shuo
```

还需要至少一次真实语音输入端到端验证：热键开始录音、说话、停止、转写并上屏。涉及麦克风、TCC 权限和前台应用，agent 不能替代。

## 7. Dry Run

```bash
cargo release <patch|minor|0.1.0> --dry-run
```

展示并确认：

- 旧版本和新版本。
- release commit message：`release: vX.Y.Z`。
- tag：`vX.Y.Z`。
- 将 push 到 `origin main` 和 `origin vX.Y.Z`。
- pre-release hook 通过：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`。

## 8. Execute

再次向用户确认后执行：

```bash
cargo release <patch|minor|0.1.0> --execute
```

这会修改 `Cargo.toml`、创建 signed release commit、创建 signed tag，并 push 到 origin。失败时停下报告状态，不使用 `--no-verify`、不自动重试。

## 9. 监控 Release Workflow

```bash
gh run list --workflow=release.yml --limit=1
gh run watch <run-id>
```

构建失败时只报告关键日志和失败阶段，不自动 rerun 或修复。

Release body 由两部分组成：

- `.github/release-body.md`：固定安装和权限提醒。
- GitHub 自动生成 release notes：commit / PR / contributor 摘要。

## 10. 验证 Artifact

```bash
gh release view vX.Y.Z
rm -rf /tmp/shuo-release-check
mkdir -p /tmp/shuo-release-check
gh release download vX.Y.Z -p '*.tar.gz' -p '*.sha256' -D /tmp/shuo-release-check
cd /tmp/shuo-release-check
expected=$(cat *.sha256 | tr -d '[:space:]')
actual=$(shasum -a 256 *.tar.gz | awk '{print $1}')
test "$expected" = "$actual"
tar -tzf *.tar.gz
```

预期 tarball 结构：

```text
shuo-vX.Y.Z-aarch64-apple-darwin/
├── shuo
├── LICENSE
├── README.md
└── README.en.md
```

artifact 大小首次发布后记录在发版总结里。后续版本如果偏离基线约 50%，先停下调查。

## 11. 真实试装

用户下载并验证：

```bash
xattr -d com.apple.quarantine ./shuo 2>/dev/null
./shuo --version
./shuo doctor
```

确认版本号、权限提示、doctor 输出正常。

## 12. 出问题时

- pre-release hook 失败：修复后重新提交，从 dry-run 开始。
- tag 与 `Cargo.toml` 版本不一致：删除本地和远端错误 tag，回到发版前检查。
- CI 在 tag 后失败：如果只改 workflow，可修 main 后 rerun；如果改代码，发 patch 版本。
- Release 已发布但有严重 bug：把坏 release 标为 pre-release 或删除，然后发 patch 版本并在 changelog 标明推荐升级。
