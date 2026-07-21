//! 安装路径抽象：`shuo` 可执行文件的「supported preferred install path」单一来源。
//!
//! 业务逻辑不硬编码 `~/.local/bin/shuo`，统一经 [`InstallLayout`] 取。其余用途的
//! 路径各有专门模块：config → [`crate::config::paths`]、state → [`crate::paths`]、
//! service plist → `crate::cli::service`、runtime socket/lock → `crate::ipc` /
//! `crate::daemon`。本模块只负责 binary install 这一层。

use anyhow::Result;
use std::path::{Path, PathBuf};

/// 已安装可执行文件名（crate binary）。
const BINARY_NAME: &str = "shuo";

/// per-user 安装布局。当前 macOS/Linux 一致用 `~/.local/bin`（无 PATH 依赖、
/// 不需要 sudo）；将来跨平台差异在此集中处理，不外泄到调用方。
pub struct InstallLayout;

impl InstallLayout {
    /// per-user 可执行目录：`~/.local/bin`。`HOME` 未设置即报错（不静默退化成
    /// 根目录下的 `/.local/bin`）。
    pub fn bin_dir() -> Result<PathBuf> {
        bin_dir_from(&std::env::var("HOME").unwrap_or_default())
    }

    /// 受支持的 preferred install path：`~/.local/bin/shuo`。
    pub fn preferred_bin() -> Result<PathBuf> {
        Ok(Self::bin_dir()?.join(BINARY_NAME))
    }
}

fn bin_dir_from(home: &str) -> Result<PathBuf> {
    if home.is_empty() {
        anyhow::bail!("{}", crate::i18n::tr("cli.install.home_unset", &[]));
    }
    Ok(PathBuf::from(home).join(".local/bin"))
}

/// 规整路径用于相等比较：仅在路径存在时 canonicalize（解析 symlink/`..`），否则退回
/// 字面、区分大小写比较（不存在路径无法解析）。
fn normalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 两个路径是否指向同一可执行文件。
pub fn same_path(a: &Path, b: &Path) -> bool {
    normalize(a) == normalize(b)
}

/// `shuo update` 的安装计划。target 永远是 preferred install path（自愈地把所有人
/// 收敛到 `~/.local/bin/shuo`）；当前运行的 binary 不是它时记 `migrated_from`，
/// 由调用方在更新成功后提示用户迁移（更新 PATH、repoint service、删旧 binary），
/// 但绝不 sudo 写系统目录、也不自动删旧 binary。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub target: PathBuf,
    pub migrated_from: Option<PathBuf>,
}

/// 纯函数；preferred 由调用方注入便于测试。
pub fn plan_update_with(current_exe: &Path, preferred_bin: &Path) -> UpdatePlan {
    let migrated_from = if same_path(current_exe, preferred_bin) {
        None
    } else {
        Some(current_exe.to_path_buf())
    };
    UpdatePlan {
        target: preferred_bin.to_path_buf(),
        migrated_from,
    }
}

/// 见 [`plan_update_with`]；preferred 取自 [`InstallLayout::preferred_bin`]。
pub fn plan_update(current_exe: &Path) -> Result<UpdatePlan> {
    Ok(plan_update_with(
        current_exe,
        &InstallLayout::preferred_bin()?,
    ))
}

/// PATH 中第一个 `shuo`（用于诊断「PATH 解析到的不是 preferred bin」）。
pub fn path_first_binary() -> Option<PathBuf> {
    path_first_binary_in(&std::env::var("PATH").unwrap_or_default())
}

fn path_first_binary_in(path_var: &str) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .map(|dir| dir.join(BINARY_NAME))
        .find(|candidate| candidate.is_file())
}

/// 安装路径漂移诊断结果（纯数据，便于无 i18n 测试）。所有项都以 preferred 为基准比较，
/// 结论与「从哪运行诊断」无关——共用的 `NotPreferred` 后缀正是这个不变量的体现。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum DriftFinding {
    /// 当前运行的 binary 不是 preferred install path。
    CurrentNotPreferred {
        current: PathBuf,
        preferred: PathBuf,
    },
    /// launchd plist 指向的 binary 不是 preferred（daemon 会拉起非受支持 binary）。
    PlistNotPreferred { plist: PathBuf, preferred: PathBuf },
    /// PATH 解析到的 `shuo` 不是 preferred install path。
    PathFirstNotPreferred {
        path_first: PathBuf,
        preferred: PathBuf,
    },
}

/// 纯函数诊断：current exe / plist binary / PATH-first `shuo` 是否都收敛到 preferred bin。
pub fn diagnose_drift(
    current_exe: &Path,
    preferred_bin: &Path,
    plist_program: Option<&Path>,
    path_first: Option<&Path>,
) -> Vec<DriftFinding> {
    let mut findings = Vec::new();
    if !same_path(current_exe, preferred_bin) {
        findings.push(DriftFinding::CurrentNotPreferred {
            current: current_exe.to_path_buf(),
            preferred: preferred_bin.to_path_buf(),
        });
    }
    if let Some(plist) = plist_program {
        if !same_path(plist, preferred_bin) {
            findings.push(DriftFinding::PlistNotPreferred {
                plist: plist.to_path_buf(),
                preferred: preferred_bin.to_path_buf(),
            });
        }
    }
    if let Some(path_first) = path_first {
        if !same_path(path_first, preferred_bin) {
            findings.push(DriftFinding::PathFirstNotPreferred {
                path_first: path_first.to_path_buf(),
                preferred: preferred_bin.to_path_buf(),
            });
        }
    }
    findings
}

/// 把一条 drift finding 渲染成本地化提示行。
pub fn render_drift(finding: &DriftFinding) -> String {
    match finding {
        DriftFinding::CurrentNotPreferred { current, preferred } => crate::i18n::tr(
            "cli.install.drift_current_not_preferred",
            &[
                ("current", current.display().to_string()),
                ("preferred", preferred.display().to_string()),
            ],
        ),
        DriftFinding::PlistNotPreferred { plist, preferred } => crate::i18n::tr(
            "cli.install.drift_plist_not_preferred",
            &[
                ("plist", plist.display().to_string()),
                ("preferred", preferred.display().to_string()),
            ],
        ),
        DriftFinding::PathFirstNotPreferred {
            path_first,
            preferred,
        } => crate::i18n::tr(
            "cli.install.drift_path_first_not_preferred",
            &[
                ("path_first", path_first.display().to_string()),
                ("preferred", preferred.display().to_string()),
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_dir_from_home_builds_local_bin() {
        let dir = bin_dir_from("/home/u").unwrap();
        assert_eq!(dir, PathBuf::from("/home/u/.local/bin"));
    }

    #[test]
    fn bin_dir_errors_when_home_unset() {
        crate::i18n::init("en-US");
        let err = bin_dir_from("").unwrap_err();
        assert!(err.to_string().contains("HOME"), "{err}");
    }

    #[test]
    fn preferred_bin_is_local_bin_shuo() {
        let preferred = InstallLayout::preferred_bin().unwrap();
        assert!(
            preferred.ends_with(".local/bin/shuo"),
            "{}",
            preferred.display()
        );
    }

    #[test]
    fn same_path_compares_literal_when_not_on_disk() {
        assert!(same_path(
            Path::new("/home/u/.local/bin/shuo"),
            Path::new("/home/u/.local/bin/shuo")
        ));
        assert!(!same_path(
            Path::new("/usr/local/bin/shuo"),
            Path::new("/home/u/.local/bin/shuo")
        ));
    }

    #[test]
    fn plan_update_in_place_when_running_preferred() {
        let preferred = PathBuf::from("/home/u/.local/bin/shuo");
        let plan = plan_update_with(&preferred, &preferred);
        assert_eq!(plan.target, preferred);
        assert_eq!(plan.migrated_from, None);
    }

    #[test]
    fn plan_update_migrates_to_preferred_from_elsewhere() {
        let preferred = PathBuf::from("/home/u/.local/bin/shuo");
        let current = PathBuf::from("/usr/local/bin/shuo");

        let plan = plan_update_with(&current, &preferred);

        assert_eq!(plan.target, preferred);
        assert_eq!(plan.migrated_from, Some(current));
    }

    #[test]
    fn path_first_binary_finds_first_dir_with_executable() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-pathfirst-{}", ulid::Ulid::generate()));
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let shuo = bin_dir.join("shuo");
        std::fs::write(&shuo, b"#!/bin/sh\n").unwrap();
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();

        let path_var = std::env::join_paths([&empty, &bin_dir]).unwrap();
        let found = path_first_binary_in(path_var.to_str().unwrap());

        assert_eq!(found, Some(shuo));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn path_first_binary_none_when_absent() {
        assert_eq!(path_first_binary_in(""), None);
    }

    #[test]
    fn diagnose_drift_reports_no_findings_when_consistent() {
        let preferred = Path::new("/home/u/.local/bin/shuo");
        let findings = diagnose_drift(preferred, preferred, Some(preferred), Some(preferred));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn diagnose_drift_anchors_every_item_to_preferred() {
        let current = Path::new("/usr/local/bin/shuo");
        let preferred = Path::new("/home/u/.local/bin/shuo");
        let plist = Path::new("/opt/old/shuo");
        let path_first = Path::new("/usr/local/bin/shuo");

        let findings = diagnose_drift(current, preferred, Some(plist), Some(path_first));

        assert_eq!(
            findings,
            vec![
                DriftFinding::CurrentNotPreferred {
                    current: current.to_path_buf(),
                    preferred: preferred.to_path_buf(),
                },
                DriftFinding::PlistNotPreferred {
                    plist: plist.to_path_buf(),
                    preferred: preferred.to_path_buf(),
                },
                DriftFinding::PathFirstNotPreferred {
                    path_first: path_first.to_path_buf(),
                    preferred: preferred.to_path_buf(),
                },
            ]
        );
    }

    #[test]
    fn diagnose_drift_plist_ok_when_pointing_at_preferred_even_if_current_differs() {
        let current = Path::new("/usr/local/bin/shuo");
        let preferred = Path::new("/home/u/.local/bin/shuo");

        // plist 正确指向 preferred；只有 current 漂移，结果不依赖「从哪运行」。
        let findings = diagnose_drift(current, preferred, Some(preferred), None);

        assert_eq!(
            findings,
            vec![DriftFinding::CurrentNotPreferred {
                current: current.to_path_buf(),
                preferred: preferred.to_path_buf(),
            }]
        );
    }

    #[test]
    fn diagnose_drift_skips_optional_inputs_when_absent() {
        let current = Path::new("/home/u/.local/bin/shuo");
        let preferred = Path::new("/home/u/.local/bin/shuo");

        let findings = diagnose_drift(current, preferred, None, None);

        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn render_drift_includes_paths() {
        crate::i18n::init("en-US");
        let line = render_drift(&DriftFinding::PlistNotPreferred {
            plist: PathBuf::from("/opt/old/shuo"),
            preferred: PathBuf::from("/home/u/.local/bin/shuo"),
        });
        assert!(line.contains("/opt/old/shuo"), "{line}");
        assert!(line.contains("/home/u/.local/bin/shuo"), "{line}");
    }
}
