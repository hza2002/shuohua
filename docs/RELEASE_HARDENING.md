# 首次公开发版前的加固清单

> **谁读这份文档**：当 shuohua 准备做首次面向外部用户的正式发版时，主控（人 + agent）按本清单**一次性**走完 7 个步骤，再回到 [docs/RELEASE.md](RELEASE.md) 执行真正的发版流程。
>
> **Agent 读法**：本文档是顺序执行的清单。每个步骤含【依赖】【动作】【验证】【失败回退】。**每个步骤都标【需用户确认】**——agent 必须把当前掌握的信息呈现给用户，等用户拍板再执行。本文档涉及的所有动作均**不可在 agent 自动模式下静默完成**。

## 为什么需要本文档

[docs/RELEASE.md](RELEASE.md) 描述的是**每次发版**的操作流程。本文档描述的是**第一次正式发版前**需要做的**一次性**加固，包括：

- 把仓库推到 GitHub（当前还是纯本地仓库）
- 启用提交签名 + 给 main 分支自动开 GPG 签名
- 让 release commit 和 v\* tag 强制签名
- 把"agent 不许碰 main、不许触发发版"的硬规则写进 [CLAUDE.md](../CLAUDE.md)
- 在 GitHub 远端开 branch / tag protection，做服务端兜底

这些做完后，[docs/RELEASE.md](RELEASE.md) 的 10 步发版流程才真正安全。

---

## 当前状态基线（截至 2026-06-20）

| 项 | 状态 |
|---|---|
| `LICENSE` / `README.md` / `release.toml` / `.github/workflows/release.yml` / `.github/release-body.md` 已在 main | ✅ 已就绪 |
| `docs/RELEASE.md` 已写好（含 10 步发版流程） | ✅ 已就绪 |
| `cargo-release` 工具本地已装 | ✅ 已就绪 |
| 仓库 `commit.gpgsign = false` / `tag.gpgsign = false` 本地 override | ⚠️ 开发期临时关闭，加固时撤销 |
| `release.toml` 中 `sign-commit = false` / `sign-tag = false` | ⚠️ 加固时改 true |
| Git remote `origin` | ❌ 未配置 |
| 仓库历史从未 push 到 GitHub | ❌ 未 push |
| `.git/config` 中 `includeIf "onbranch:main"` 自动签名钩子 | ❌ 未装 |
| `CLAUDE.md` 中 agent 工作流硬规则 | ❌ 未写 |
| GitHub branch protection（main 要求签名） | ❌ 未开 |
| GitHub tag protection（v\* 限制 pusher）| ❌ 未开 |

加固完成后所有项应全为 ✅。

---

## 何时执行本清单

任意一个触发：

- 决定做首次面向外部用户的正式发版（v0.1.0 或更高的 first public release）
- 决定开始让 agent 大量参与开发，需要明确"agent 不能直接发版"的护栏
- 准备把仓库公开到 GitHub（即使还不发版）

---

## 执行原则

- **严格按步骤顺序**。每个步骤有【依赖】，前置不满足不要跳。
- **每步【需用户确认】**。agent 必须明示要执行什么、为什么、可能的副作用，等用户拍板。
- **任何步骤失败 → 停下报告**。不要自动修复、不要自动 retry、不要 `--force` / `--no-verify`。
- **本文档涉及 GitHub 远端的操作**：可以用 `gh` CLI 完成，不强制开 GitHub UI。所有 `gh` 命令需要你的账号已 `gh auth login`。

---

## 步骤 1: 配置 git remote 并首次 push 【需用户确认】

**依赖**：仓库已在 main 分支，working tree 干净。

**为什么**：当前是纯本地仓库，没有 remote。后面所有步骤（branch protection、tag protection、CI workflow 触发）都依赖远端存在。

**动作**：

```bash
# 1.1 检查当前 remote 状态
cd /Users/ghot/repo/shuohua
git remote -v   # 预期：空输出

# 1.2 添加 origin（用户确认仓库地址）
git remote add origin git@github.com:HuZiang/shuohua.git

# 1.3 首次 push 全部 commit
git push -u origin main

# 1.4 验证远端可见
gh repo view HuZiang/shuohua --json defaultBranchRef,pushedAt
```

⚠️ **确认前问用户**：
- GitHub 仓库 `HuZiang/shuohua` 已经在远端**新建好**了吗（空仓库即可）？没建好的话先建 → `gh repo create HuZiang/shuohua --private` 或 `--public`
- 仓库 visibility 是 public 还是 private？影响下游 Homebrew / 公开下载

**验证**：
- `git push` 成功无报错
- `gh repo view` 显示 default branch 是 main，commit 数量与本地 `git log --oneline | wc -l` 一致

**失败回退**：
- remote 加错 URL → `git remote remove origin` 重来
- push 被 reject（远端已有内容）→ 停下让用户决定是否 force（**默认不 force**，先查清远端是什么）

---

## 步骤 2: 撤销本仓库签名 override 【需用户确认】

**依赖**：步骤 1 完成。用户全局 `commit.gpgsign = true`（用 `git config --global --get commit.gpgsign` 确认）。

**为什么**：开发期本仓库显式关掉了签名（避免 GPG 拖慢 commit 速度）。加固后要让 main 上的 commit 都签名，先撤销这个 override。

**动作**：

```bash
# 2.1 确认全局默认是签名（用户机器配置）
git config --global --get commit.gpgsign     # 预期：true
git config --global --get tag.gpgsign         # 预期：true
git config --global --get user.signingkey    # 预期：GPG key ID

# 2.2 撤销本仓库 override
git config --local --unset commit.gpgsign 2>/dev/null
git config --local --unset tag.gpgsign 2>/dev/null

# 2.3 验证：现在继承全局
git config --get commit.gpgsign   # 预期：true
git config --get tag.gpgsign       # 预期：true
```

**验证**：上面三个 `--get` 输出都是 `true`。

**失败回退**：用户没配全局 GPG → 停下让用户先配（`man gpg-agent` / 自查），不要在本文档里替用户引导。

---

## 步骤 3: 装 includeIf 让 main 自动签名 【需用户确认】

**依赖**：步骤 2 完成。git 版本 ≥ 2.36（`git --version` 确认；macOS 26 默认满足）。

**为什么**：开发期 agent 在 `feat/*` 分支上 commit 不希望被 GPG passphrase 卡住。`includeIf "onbranch:main"` 让 git 在切到 main 时自动启用签名、切回 feat 时关闭。

**动作**：

```bash
# 3.1 在 .git/config 末尾追加 includeIf 段
cat >> /Users/ghot/repo/shuohua/.git/config <<'EOF'

[includeIf "onbranch:main"]
    path = config.signed
EOF

# 3.2 写被引用的 config.signed 文件
cat > /Users/ghot/repo/shuohua/.git/config.signed <<'EOF'
[commit]
    gpgsign = true
[tag]
    gpgsign = true
EOF

# 3.3 同时写本仓库 local 为 false（feat 默认不签名）
git config --local commit.gpgsign false
git config --local tag.gpgsign false
```

⚠️ **`.git/config` 和 `.git/config.signed` 不进 git**。每台开发机克隆后都要重新装一次。把"如何装"写进 [CLAUDE.md](../CLAUDE.md) onboarding（步骤 5 会做）。

**验证**：

```bash
# 在 main 上：应该签
git checkout main
git config --get commit.gpgsign   # 预期：true

# 切到一个 feat 分支：应该不签
git checkout -b feat/_test
git config --get commit.gpgsign   # 预期：false
git checkout main
git branch -d feat/_test
```

如果切分支后值不变，说明 git 版本太老或 includeIf 语法错。

**失败回退**：删 `.git/config.signed` + 从 `.git/config` 删掉 includeIf 段，恢复原状。

---

## 步骤 4: 改 release.toml 强制签名 【需用户确认】

**依赖**：步骤 3 完成（验证 main 上能正常签名）。

**为什么**：`cargo release` 产生的 release commit 和 v\* tag 是发版的最终凭证。即使用户忘了切 main，release.toml 也应该兜底强制签名。

**动作**：

```bash
# 4.1 用 sed 或编辑器把两行 false 改成 true
# 在 main 分支上做（changes 直接进 main，会自动签名）
git checkout main

# 编辑 /Users/ghot/repo/shuohua/release.toml
# sign-commit = false  →  sign-commit = true
# sign-tag    = false  →  sign-tag    = true

# 4.2 验证 cargo release 配置仍能解析
cargo release config | grep -E 'sign-commit|sign-tag'
# 预期输出：
#   sign_commit = true
#   sign_tag = true

# 4.3 commit 并 push
git add release.toml
git commit -m "chore: enable signed commits and tags in cargo-release"
# ↑ 因为 includeIf，这个 commit 自动签名，会提示 GPG passphrase
git log -1 --show-signature   # 验证：Good signature from <你的名字>
git push origin main
```

**验证**：
- `cargo release config` 输出含 `sign_commit = true` 和 `sign_tag = true`
- `git log -1 --show-signature` 显示 Good signature
- GitHub 网页上看这个 commit 显示 "Verified" 徽章

**失败回退**：
- 签名失败（passphrase 错 / agent 没启动）→ 解决 GPG 环境后重试，不要 unset gpgsign 跳过
- `cargo release config` 报错 → 检查 toml 语法

---

## 步骤 5: 更新 CLAUDE.md 加 agent 工作流硬规则 【需用户确认】

**依赖**：无强依赖（可以和步骤 4 合并 commit，但建议单独 commit 便于回查）。

**为什么**：CLAUDE.md 是给所有未来 agent 读的根指令。明确写"不许碰 main"是技术锁之外的纪律层。

**动作**：在 `/Users/ghot/repo/shuohua/CLAUDE.md` 加一节（建议放在 `## Git workflow` 后面）：

````markdown
## Agent 工作流硬规则（不可违反）

发版前加固已完成（见 [docs/RELEASE_HARDENING.md](docs/RELEASE_HARDENING.md)），以下规则强制生效：

- agent 只在 `feat/*` 或 `agent/*` 分支 commit；**不准 `git checkout main`**
- agent **不准** 做这些动作：
  - `git merge`（任何方向）
  - `git push`（任何分支，包括 feat）
  - `git tag`（特别是 `v*`）
  - `cargo release ... --execute`
  - 修改 `release.toml`、`.github/workflows/release.yml`、`docs/RELEASE.md`、`docs/RELEASE_HARDENING.md`
- agent 完成开发后报告"请 review feat/X"，由用户切 main → squash merge → push → 发版
- `release.toml` 里 `allow-branch = ["main"]` 是技术锁，agent 在 feat 分支跑 cargo-release 会被工具拒绝。**不要试图绕过**
- main 上的 commit 由 includeIf 自动 GPG 签名，需要用户输入 passphrase——agent 没有 passphrase

### 新机器 / 新克隆后一次性 setup

让 main 自动 GPG 签名（仅本机生效，`.git/config` 不进 git）：

```bash
cat >> .git/config <<'EOF'

[includeIf "onbranch:main"]
    path = config.signed
EOF
cat > .git/config.signed <<'EOF'
[commit]
    gpgsign = true
[tag]
    gpgsign = true
EOF
git config --local commit.gpgsign false
git config --local tag.gpgsign false
```

验证：`git checkout main && git config --get commit.gpgsign` 应输出 `true`；切回 feat 分支输出 `false`。
````

```bash
# commit 并 push
git checkout main   # 已在
git add CLAUDE.md
git commit -m "docs: add agent workflow hard rules in CLAUDE.md"
git push origin main
```

**验证**：CLAUDE.md 包含上面那一节；commit 在 GitHub 显示 Verified。

**失败回退**：撤回 commit（`git reset --hard HEAD~1` + `git push --force-with-lease`）。**force push 必须用户亲自做，agent 永远不许 force push**。

---

## 步骤 6: GitHub branch protection 【需用户确认】

**依赖**：步骤 1 完成（远端存在）。`gh auth login` 已完成（`gh auth status` 确认）。

**为什么**：服务端兜底——即使 agent 绕过本地所有约束 push 到 main，远端会因 commit 未签名而 reject。这是最后一道关卡。

**动作**：分两步——基础 protection + 单独开 required_signatures（GitHub 把 required_signatures 设计成独立子端点）。

```bash
# 6.1 检查 gh auth 状态
gh auth status

# 6.2 开基础 branch protection（不含 required_signatures）
gh api -X PUT "repos/HuZiang/shuohua/branches/main/protection" --input - <<'EOF'
{
  "required_status_checks": null,
  "enforce_admins": true,
  "required_pull_request_reviews": null,
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false
}
EOF

# 6.3 单独开 required_signatures（必须独立调用）
gh api -X POST "repos/HuZiang/shuohua/branches/main/protection/required_signatures"
```

**验证**：

```bash
# 检查 required_signatures
gh api "repos/HuZiang/shuohua/branches/main/protection/required_signatures" \
  --jq '.enabled'   # 预期：true

# 检查其他 protection 项
gh api "repos/HuZiang/shuohua/branches/main/protection" \
  --jq '{admins: .enforce_admins.enabled, force_push: .allow_force_pushes.enabled, deletions: .allow_deletions.enabled}'
# 预期：admins=true, force_push=false, deletions=false
```

**测试**（**可选**，会留下一个测试 commit，跳过更稳）：尝试 push 一个未签名 commit 应该被 reject。

**失败回退**：

```bash
# 撤销 required_signatures
gh api -X DELETE "repos/HuZiang/shuohua/branches/main/protection/required_signatures"

# 撤销整个 branch protection
gh api -X DELETE "repos/HuZiang/shuohua/branches/main/protection"
```

⚠️ `enforce_admins = true` 意味着你自己也受限。一旦开启，你也不能 force push main、不能 push 未签名 commit。**这是有意的**——防止自己手滑或 agent 假冒你 push 漏签的 commit。

---

## 步骤 7: GitHub tag protection via Rulesets 【需用户确认】

**依赖**：步骤 1 完成。

**为什么**：限制 `v*` tag 的创建 / 删除 / push。配合步骤 6 的签名要求，发版触发器（v\* tag push）被双重保护。

⚠️ **API 选择**：GitHub 旧的 `tags/protection` 端点已 deprecated（2024 起逐步淘汰），用新的 **Rulesets API**。Rulesets 更强大、有 GitHub 长期支持承诺。

**动作**：

```bash
# 7.1 创建一个 ruleset 保护 v* tag
gh api -X POST "repos/HuZiang/shuohua/rulesets" --input - <<'EOF'
{
  "name": "Protect v* release tags",
  "target": "tag",
  "enforcement": "active",
  "conditions": {
    "ref_name": {
      "include": ["refs/tags/v*"],
      "exclude": []
    }
  },
  "rules": [
    { "type": "creation" },
    { "type": "deletion" },
    { "type": "non_fast_forward" }
  ],
  "bypass_actors": []
}
EOF
```

`rules` 里三项：
- `creation` — 限制谁能创建匹配 tag（默认仅 admin）
- `deletion` — 防止误删 release tag
- `non_fast_forward` — 防止 force push 已发布的 tag

`bypass_actors: []` 意味着**没人能绕过**，包括 admin（你自己也受限）。这是有意的——防止手滑 force push 已发布的 v\* tag 把已下载的用户搞糊涂。

**验证**：

```bash
# 列出本仓库所有 rulesets，应能看到刚创建的
gh api "repos/HuZiang/shuohua/rulesets" --jq '.[] | {id, name, target, enforcement}'
# 预期至少一行：{id, name: "Protect v* release tags", target: "tag", enforcement: "active"}

# 取记下来的 id 看详情
gh api "repos/HuZiang/shuohua/rulesets/<id>" \
  --jq '{name, conditions: .conditions.ref_name.include, rules: [.rules[].type]}'
```

**失败回退**：

```bash
# 用上面输出的 id
gh api -X DELETE "repos/HuZiang/shuohua/rulesets/<id>"
```

---

## 完成验证

所有 7 步完成后，跑一遍校验：

```bash
cd /Users/ghot/repo/shuohua

# 本地
git remote -v | grep origin                                          # ✅ 有 origin
git config --get commit.gpgsign                                      # ✅ true (在 main 上)
grep -E 'sign-commit|sign-tag' release.toml                          # ✅ 都是 true
grep -c "Agent 工作流硬规则" CLAUDE.md                                # ✅ 1
test -f .git/config.signed && echo "includeIf installed"             # ✅

# 远端
gh api "repos/HuZiang/shuohua/branches/main/protection/required_signatures" \
  --jq '.enabled'                                                    # ✅ true
gh api "repos/HuZiang/shuohua/rulesets" \
  --jq '[.[] | select(.target == "tag")] | length'                   # ✅ ≥ 1
```

七项全 ✅ → 加固完成，可以进入 [docs/RELEASE.md](RELEASE.md) 走真实发版流程。

---

## 未在本清单范围（明确不做）

下列项各自有更长的决策路径，**不要塞进本次加固**：

- **Apple Developer ID 签名 / 公证**：付费决策，触发条件见 [docs/RELEASE.md §6.1](RELEASE.md)
- **Homebrew tap 仓库**：触发条件见 [docs/RELEASE.md §6.2](RELEASE.md)
- **跨平台扩展（Linux / Windows）**：触发条件见 [docs/RELEASE.md §6.3](RELEASE.md)
- **`shuo doctor` 增加 deep link 改进**：单独 issue / PR 跟，不阻塞发版
- **CI 主干 PR 验证 workflow**：现在没有外部贡献者，暂不需要

---

## 关于本文档自身的维护

- 本文档**只在结构变化时改**（比如加固完成后某项失效需要再补救）。
- 加固完成后，把"当前状态基线"那张表全部标 ✅，加一个日期：**"完成于 YYYY-MM-DD"**。
- 如果未来撤销了某项（例如改为 cargo-dist 后 release.toml 不再用），**不要删本文档**——加注"已废弃，被 X 替代"，保留历史。
