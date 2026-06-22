use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/hza2002/shuohua/releases/latest";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedAssets {
    pub version: semver::Version,
    pub tarball_url: String,
    pub sha256_url: String,
}

pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .timeout(HTTP_REQUEST_TIMEOUT)
        .build()
        .context("create update HTTP client")
}

pub async fn fetch_latest(client: &reqwest::Client) -> Result<GitHubRelease> {
    client
        .get(LATEST_RELEASE_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(
            reqwest::header::USER_AGENT,
            concat!("shuo/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await
        .context("check latest GitHub release")?
        .error_for_status()
        .context("GitHub latest release request failed")?
        .json::<GitHubRelease>()
        .await
        .context("parse GitHub latest release response")
}

pub async fn download_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            concat!("shuo/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await
        .with_context(|| format!("download {url}"))?
        .error_for_status()
        .with_context(|| format!("download {url} failed"))?;
    if let Some(length) = response.content_length() {
        ensure_download_size(length, url)?;
    }
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("read downloaded body from {url}"))?;
    ensure_download_size(bytes.len() as u64, url)?;
    Ok(bytes.to_vec())
}

fn ensure_download_size(bytes: u64, url: &str) -> Result<()> {
    if bytes > MAX_DOWNLOAD_BYTES {
        anyhow::bail!(
            "download {url} is too large: {bytes} bytes exceeds {} bytes",
            MAX_DOWNLOAD_BYTES
        );
    }
    Ok(())
}

pub fn select_assets(release: &GitHubRelease, target: &str) -> Result<SelectedAssets> {
    let version = crate::cli::app::version::parse_release_tag(&release.tag_name)
        .with_context(|| format!("invalid release tag {}", release.tag_name))?;
    let prefix = format!("shuo-v{version}-{target}.tar.gz");
    let sha = format!("{prefix}.sha256");

    let tarball_url = release
        .assets
        .iter()
        .find(|asset| asset.name == prefix)
        .map(|asset| asset.browser_download_url.clone())
        .with_context(|| format!("latest release has no asset {prefix}"))?;
    let sha256_url = release
        .assets
        .iter()
        .find(|asset| asset.name == sha)
        .map(|asset| asset.browser_download_url.clone())
        .with_context(|| format!("latest release has no checksum asset {sha}"))?;

    Ok(SelectedAssets {
        version,
        tarball_url,
        sha256_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_matching_tarball_and_checksum() {
        let release = GitHubRelease {
            tag_name: "v0.2.0".to_string(),
            assets: vec![
                GitHubAsset {
                    name: "shuo-v0.2.0-aarch64-apple-darwin.tar.gz".to_string(),
                    browser_download_url: "https://example/tar".to_string(),
                },
                GitHubAsset {
                    name: "shuo-v0.2.0-aarch64-apple-darwin.tar.gz.sha256".to_string(),
                    browser_download_url: "https://example/sha".to_string(),
                },
            ],
        };

        let selected = select_assets(&release, "aarch64-apple-darwin").unwrap();

        assert_eq!(selected.version, semver::Version::parse("0.2.0").unwrap());
        assert_eq!(selected.tarball_url, "https://example/tar");
        assert_eq!(selected.sha256_url, "https://example/sha");
    }

    #[test]
    fn errors_when_platform_asset_is_missing() {
        let release = GitHubRelease {
            tag_name: "v0.2.0".to_string(),
            assets: vec![],
        };

        let err = select_assets(&release, "aarch64-apple-darwin").unwrap_err();
        assert!(err.to_string().contains("no asset"), "{err:#}");
    }

    #[test]
    fn rejects_downloads_over_size_limit() {
        let err = ensure_download_size(MAX_DOWNLOAD_BYTES + 1, "https://example").unwrap_err();
        assert!(err.to_string().contains("too large"), "{err:#}");
    }
}
