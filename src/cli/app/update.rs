use anyhow::{Context, Result};
use semver::Version;
use std::path::PathBuf;

use super::platform::UpdatePlatform;
use super::version::UpdateDecision;

pub struct UpdateOutcome {
    pub from: Version,
    pub to: Version,
    pub installed_path: PathBuf,
    /// 当前运行的 binary 不是 preferred 路径时，记录旧路径，提示用户迁移。
    pub migrated_from: Option<PathBuf>,
}

pub fn current_version() -> Result<Version> {
    Version::parse(env!("CARGO_PKG_VERSION")).map_err(Into::into)
}

pub fn ensure_update_allowed(
    current: &Version,
    latest: &Version,
    allow_major: bool,
) -> Result<Option<UpdateDecision>> {
    match super::version::decide(current, latest, allow_major) {
        UpdateDecision::Current => Ok(None),
        UpdateDecision::RefuseMajor { from, to } => {
            anyhow::bail!(
                "{}",
                crate::i18n::tr(
                    "cli.app.update.refuse_major",
                    &[("current", from.to_string()), ("latest", to.to_string())]
                )
            )
        }
        decision @ UpdateDecision::Update { .. } => Ok(Some(decision)),
    }
}

pub async fn run(args: crate::cli::app::UpdateArgs) -> Result<()> {
    println!("{}", crate::i18n::tr("cli.app.update.checking", &[]));
    let client = crate::cli::app::release::http_client()?;
    let platform = crate::cli::app::platform::current();
    let target = platform.artifact_target()?;
    let current = current_version()?;
    let release = crate::cli::app::release::fetch_latest(&client).await?;
    let selected = crate::cli::app::release::select_assets(&release, target)?;

    let Some(decision) = ensure_update_allowed(&current, &selected.version, args.allow_major)?
    else {
        println!(
            "{}",
            crate::i18n::tr(
                "cli.app.update.current",
                &[("version", current.to_string())]
            )
        );
        return Ok(());
    };

    if args.check {
        println!("{}", update_check_message(&current, decision));
        return Ok(());
    }

    let UpdateDecision::Update { from, to } = decision else {
        unreachable!("ensure_update_allowed only returns installable update decisions");
    };
    let outcome = install_update(&client, &platform, target, &selected, from, to).await?;

    println!(
        "{}",
        crate::i18n::tr(
            "cli.app.update.updated",
            &[
                ("current", outcome.from.to_string()),
                ("latest", outcome.to.to_string()),
                ("path", outcome.installed_path.display().to_string())
            ]
        )
    );
    if let Some(previous) = &outcome.migrated_from {
        println!(
            "{}",
            crate::i18n::tr(
                "cli.app.update.migrated",
                &[
                    ("preferred", outcome.installed_path.display().to_string()),
                    ("previous", previous.display().to_string()),
                ]
            )
        );
    }
    println!("{}", crate::i18n::tr("cli.app.update.restart_hint", &[]));
    println!("{}", update_permission_hint(&outcome.installed_path));
    // 以装好的 preferred binary 为基准报漂移：catch「update 原地成功，但 plist / PATH
    // 仍指向别的旧 binary」——restart 后 daemon 会拉起错的那个。current 用 installed_path，
    // 故不重复迁移提示里的 CurrentNotPreferred。
    for finding in post_update_drift(&outcome.installed_path) {
        println!("{}", crate::install::render_drift(&finding));
    }
    Ok(())
}

fn post_update_drift(installed: &std::path::Path) -> Vec<crate::install::DriftFinding> {
    let plist = crate::cli::service::plist_program();
    let path_first = crate::install::path_first_binary();
    crate::install::diagnose_drift(
        installed,
        installed,
        plist.as_deref(),
        path_first.as_deref(),
    )
}

fn update_permission_hint(installed_path: &std::path::Path) -> String {
    crate::i18n::tr(
        "cli.app.update.permission_hint",
        &[("path", installed_path.display().to_string())],
    )
}

fn update_check_message(current: &Version, decision: UpdateDecision) -> String {
    match decision {
        UpdateDecision::Update { to, .. } => crate::i18n::tr(
            "cli.app.update.available",
            &[("current", current.to_string()), ("latest", to.to_string())],
        ),
        UpdateDecision::Current => crate::i18n::tr(
            "cli.app.update.current",
            &[("version", current.to_string())],
        ),
        UpdateDecision::RefuseMajor { .. } => {
            unreachable!("refused updates are returned as errors before rendering")
        }
    }
}

async fn install_update(
    client: &reqwest::Client,
    platform: &impl UpdatePlatform,
    target: &str,
    selected: &crate::cli::app::release::SelectedAssets,
    from: Version,
    to: Version,
) -> Result<UpdateOutcome> {
    println!(
        "{}",
        crate::i18n::tr("cli.app.update.downloading", &[("version", to.to_string())])
    );
    let tarball = crate::cli::app::release::download_bytes(client, &selected.tarball_url).await?;
    let checksum = crate::cli::app::release::download_bytes(client, &selected.sha256_url).await?;
    let checksum = String::from_utf8(checksum).context("checksum file is not UTF-8")?;
    crate::cli::app::archive::verify_sha256(&tarball, &checksum)?;

    let temp_dir = std::env::temp_dir().join(format!("shuohua-update-{}", ulid::Ulid::new()));
    let _temp_dir_cleanup = TempDirCleanup::new(temp_dir.clone());
    let expected_binary_path = PathBuf::from(format!("shuo-v{to}-{target}/shuo"));
    let extracted =
        crate::cli::app::archive::extract_shuo_binary(&tarball, &temp_dir, &expected_binary_path)?;
    platform.prepare_executable(&extracted)?;
    let plan = crate::install::plan_update(&platform.current_exe()?)?;
    platform.install_executable(&extracted, &plan.target)?;

    Ok(UpdateOutcome {
        from,
        to,
        installed_path: plan.target,
        migrated_from: plan.migrated_from,
    })
}

struct TempDirCleanup {
    path: PathBuf,
}

impl TempDirCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_parses_package_version() {
        current_version().unwrap();
    }

    #[test]
    fn ensure_update_allowed_returns_none_when_current() {
        let current = Version::parse("0.2.0").unwrap();
        let latest = Version::parse("0.2.0").unwrap();

        assert!(ensure_update_allowed(&current, &latest, false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn ensure_update_allowed_refuses_major() {
        crate::i18n::init("en-US");
        let current = Version::parse("0.9.0").unwrap();
        let latest = Version::parse("1.0.0").unwrap();

        let err = ensure_update_allowed(&current, &latest, false).unwrap_err();
        assert!(err.to_string().contains("--allow-major"), "{err:#}");
    }

    #[test]
    fn update_check_message_reports_available_version() {
        crate::i18n::init("en-US");
        let current = Version::parse("0.2.0").unwrap();
        let latest = Version::parse("0.3.0").unwrap();
        let decision = ensure_update_allowed(&current, &latest, false)
            .unwrap()
            .unwrap();

        let message = update_check_message(&current, decision);

        assert!(message.contains("update available"), "{message}");
        assert!(message.contains("0.2.0"), "{message}");
        assert!(message.contains("0.3.0"), "{message}");
    }

    #[test]
    fn update_permission_hint_names_installed_binary_and_accessibility() {
        crate::i18n::init("en-US");

        let preferred = crate::install::InstallLayout::preferred_bin().unwrap();
        let hint = update_permission_hint(&preferred);

        assert!(hint.contains(&preferred.display().to_string()), "{hint}");
        assert!(hint.contains("Accessibility"), "{hint}");
        assert!(hint.contains("shuo service restart"), "{hint}");
    }

    #[test]
    fn temp_dir_cleanup_removes_directory_on_drop() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-update-cleanup-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shuo"), b"binary").unwrap();

        drop(TempDirCleanup::new(dir.clone()));

        assert!(!dir.exists());
    }
}
