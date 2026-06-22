use anyhow::{Context, Result};
use semver::Version;
use std::path::PathBuf;

use super::platform::UpdatePlatform;
use super::version::UpdateDecision;

pub struct UpdateOutcome {
    pub from: Version,
    pub to: Version,
    pub installed_path: PathBuf,
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

    let Some(UpdateDecision::Update { from, to }) =
        ensure_update_allowed(&current, &selected.version, args.allow_major)?
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
    println!("{}", crate::i18n::tr("cli.app.update.restart_hint", &[]));
    Ok(())
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
    let installed_path = platform.replace_current_exe(&extracted)?;

    Ok(UpdateOutcome {
        from,
        to,
        installed_path,
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
    fn temp_dir_cleanup_removes_directory_on_drop() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-update-cleanup-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shuo"), b"binary").unwrap();

        drop(TempDirCleanup::new(dir.clone()));

        assert!(!dir.exists());
    }
}
