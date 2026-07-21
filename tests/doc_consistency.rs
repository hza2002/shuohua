//! 防文档/基础设施漂移：内部链接、顶层模块登记、Rust stable 策略同步。
//! 逐文件树已去除，其余契约尽量靠代码自身。

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 收集 docs/ + 顶层 README.md/CLAUDE.md 的 markdown 文件；跳过 archive/superpowers
/// （归档/本地草稿，允许 stale 链接）。
fn markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![
        root.join("README.md"),
        root.join("CLAUDE.md"),
        root.join("CHANGELOG.md"),
    ];
    collect_md(&root.join("docs"), &mut out);
    out.into_iter().filter(|p| p.exists()).collect()
}

fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "archive" || name == "superpowers" {
                continue;
            }
            collect_md(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

/// 抽取一行外（非代码围栏内）的 markdown 链接目标。
fn link_targets(body: &str) -> Vec<String> {
    let mut targets = vec![];
    let mut in_fence = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut i = 0;
        while let Some(rel) = line[i..].find("](") {
            let start = i + rel + 2;
            if let Some(end_rel) = line[start..].find(')') {
                let end = start + end_rel;
                targets.push(line[start..end].to_string());
                i = end + 1;
            } else {
                break;
            }
        }
    }
    targets
}

#[test]
fn doc_internal_links_resolve() {
    let root = repo_root();
    let mut broken = vec![];
    for file in markdown_files(&root) {
        let body = fs::read_to_string(&file).unwrap();
        let dir = file.parent().unwrap();
        for target in link_targets(&body) {
            // 只校验仓库内相对路径链接
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
                || target.starts_with('#')
            {
                continue;
            }
            let path_part = target.split('#').next().unwrap_or("");
            if path_part.is_empty() {
                continue;
            }
            if !dir.join(path_part).exists() {
                broken.push(format!(
                    "{} → {}",
                    file.strip_prefix(&root).unwrap().display(),
                    target
                ));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "broken doc links:\n{}",
        broken.join("\n")
    );
}

#[test]
fn top_level_modules_are_documented() {
    let root = repo_root();
    let arch = fs::read_to_string(root.join("docs/architecture.md")).unwrap();
    let mut missing = vec![];
    for entry in fs::read_dir(root.join("src")).unwrap().flatten() {
        if entry.path().is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            // 顶层树以 `name/` 形式列出；用尾斜杠匹配避免子串误判（cli ⊂ client）。
            if !arch.contains(&format!("{name}/")) {
                missing.push(name);
            }
        }
    }
    assert!(
        missing.is_empty(),
        "src/ 顶层模块未在 docs/architecture.md 登记：{missing:?}"
    );
}

#[test]
fn rust_stable_strategy_matches_local_ci_and_release() {
    let root = repo_root();
    for workflow in ["ci.yml", "release.yml"] {
        let body = fs::read_to_string(root.join(".github/workflows").join(workflow)).unwrap();
        assert!(
            body.contains("dtolnay/rust-toolchain@stable"),
            "{workflow} 必须跟随最新 Rust stable"
        );
        assert!(
            body.contains("components: clippy, rustfmt"),
            "{workflow} 必须安装 clippy 和 rustfmt"
        );
    }

    let makefile = fs::read_to_string(root.join("Makefile")).unwrap();
    assert!(
        makefile.contains("rustup update stable"),
        "make check 必须先更新 Rust stable"
    );
    for command in [
        "$(CARGO) +stable fmt --check",
        "$(CARGO) +stable clippy --locked --all-targets -- -D warnings",
        "$(CARGO) +stable test --locked",
    ] {
        assert!(makefile.contains(command), "Makefile 缺少 {command}");
    }

    let release = fs::read_to_string(root.join("release.toml")).unwrap();
    assert!(
        release.contains("rustup update stable"),
        "cargo-release hook 必须先更新 Rust stable"
    );
}
