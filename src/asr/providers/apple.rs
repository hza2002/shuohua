//! Apple SpeechAnalyzer provider (macOS 26).

use crate::asr::types::*;
use crate::config::asr::apple::{load_config_with_overrides, AppleConfig};
use async_trait::async_trait;
use serde::Deserialize;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

const HELPER_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/apple_helper"));
const DEFAULT_LANGUAGE: &str = "zh-CN";

pub use crate::config::asr::apple::config_path;

#[cfg(test)]
use crate::config::asr::apple::default_finalize_timeout_ms;

pub struct AppleProvider {
    config: AppleConfig,
}

impl AppleProvider {
    pub fn new_with_overrides(overrides: Option<&toml::value::Table>) -> anyhow::Result<Self> {
        ensure_supported_macos_version(current_macos_major_version())?;
        Ok(Self {
            config: load_config_with_overrides(overrides)?,
        })
    }

    pub fn finalize_timeout_ms(&self) -> u64 {
        self.config.finalize_timeout_ms
    }

    pub fn options(&self) -> crate::asr::providers::ProviderOptions {
        crate::asr::providers::ProviderOptions {
            idle_pause: self.config.idle_pause,
            finalize_timeout_ms: self.config.finalize_timeout_ms,
        }
    }

    pub fn runtime_check_notice(&self) -> Option<crate::asr::providers::RuntimeCheckNotice> {
        self.config
            .install_assets
            .then_some(crate::asr::providers::RuntimeCheckNotice {
                line: "asr.apple.runtime: checking Apple SpeechAnalyzer runtime; macOS may install speech assets if missing",
            })
    }

    pub async fn check_runtime(&self, ctx: SessionCtx) -> Result<(), AsrError> {
        let (mut session, mut events) = self.open(ctx).await?;
        session.send_pcm(&[], true).await?;
        let done = tokio::time::timeout(Duration::from_millis(self.finalize_timeout_ms()), async {
            while let Some(event) = events.recv().await {
                match event {
                    AsrEvent::Done => return Ok(()),
                    AsrEvent::Error { err } => return Err(err),
                    AsrEvent::Partial { .. }
                    | AsrEvent::Segment { .. }
                    | AsrEvent::Final { .. } => {}
                }
            }
            Err(AsrError::Protocol("apple helper closed before done".into()))
        })
        .await
        .map_err(|_| AsrError::Timeout);
        let close_result = session.close().await;
        done??;
        close_result
    }
}

fn current_macos_major_version() -> u64 {
    let output = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            parse_macos_major_version(&version).unwrap_or(0)
        }
        _ => 0,
    }
}

fn ensure_supported_macos_version(major: u64) -> anyhow::Result<()> {
    if major >= 26 {
        return Ok(());
    }
    anyhow::bail!(
        "Apple ASR provider requires macOS 26 or newer; use a cloud ASR provider such as doubao on older macOS versions"
    )
}

fn parse_macos_major_version(version: &str) -> Option<u64> {
    version.trim().split('.').next()?.parse().ok()
}

#[async_trait]
impl AsrProvider for AppleProvider {
    fn name(&self) -> &str {
        "apple"
    }

    fn caps(&self) -> Caps {
        Caps {
            hotwords: true,
            max_session_secs: None,
            multilingual: true,
        }
    }

    async fn open(
        &self,
        ctx: SessionCtx,
    ) -> Result<(Box<dyn AsrSession>, mpsc::Receiver<AsrEvent>), AsrError> {
        let helper = ensure_helper_binary()?;
        let language = choose_language(&self.config, &ctx);

        let mut cmd = Command::new(helper);
        cmd.arg("--language").arg(language);
        if self.config.install_assets {
            cmd.arg("--install-assets");
        }
        if !ctx.hotwords.is_empty() {
            cmd.arg("--hotwords").arg(ctx.hotwords.join(","));
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| AsrError::Server(format!("spawn apple helper: {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AsrError::Server("apple helper stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AsrError::Server("apple helper stdout unavailable".into()))?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = line;
                    tracing::debug!("apple helper emitted stderr line");
                }
            });
        }

        let (evt_tx, evt_rx) = mpsc::channel(64);
        tokio::spawn(read_helper_events(stdout, evt_tx));

        Ok((
            Box::new(AppleSession {
                stdin: Some(stdin),
                child: Some(child),
            }),
            evt_rx,
        ))
    }
}

pub struct AppleSession {
    stdin: Option<ChildStdin>,
    child: Option<Child>,
}

#[async_trait]
impl AsrSession for AppleSession {
    async fn send_pcm(&mut self, pcm: &[i16], is_last: bool) -> Result<(), AsrError> {
        let Some(stdin) = self.stdin.as_mut() else {
            if is_last {
                return Ok(());
            }
            return Err(AsrError::Network(
                "apple helper stdin already closed".into(),
            ));
        };
        let frame = encode_pcm_frame(pcm, is_last);
        stdin
            .write_all(&frame)
            .await
            .map_err(|e| AsrError::Network(format!("write apple pcm: {e}")))?;
        if is_last {
            stdin
                .shutdown()
                .await
                .map_err(|e| AsrError::Network(format!("close apple stdin: {e}")))?;
            self.stdin = None;
        }
        Ok(())
    }

    async fn close(mut self: Box<Self>) -> Result<(), AsrError> {
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }
}

async fn read_helper_events(stdout: tokio::process::ChildStdout, evt_tx: mpsc::Sender<AsrEvent>) {
    let started_at = Instant::now();
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                let _ = evt_tx
                    .send(AsrEvent::Error {
                        err: AsrError::Protocol("apple helper exited before done".into()),
                    })
                    .await;
                return;
            }
            Err(e) => {
                let _ = evt_tx
                    .send(AsrEvent::Error {
                        err: AsrError::Protocol(format!("read apple helper event: {e}")),
                    })
                    .await;
                return;
            }
        };

        match parse_helper_event(&line) {
            Ok(HelperEvent::Partial { text, seq }) => {
                let _ = evt_tx.send(AsrEvent::Partial { text, seq }).await;
            }
            Ok(HelperEvent::Segment {
                text,
                start_ms,
                end_ms,
            }) => {
                let _ = evt_tx
                    .send(AsrEvent::Segment {
                        text,
                        started_at: started_at + Duration::from_millis(start_ms),
                        ended_at: started_at + Duration::from_millis(end_ms),
                    })
                    .await;
            }
            Ok(HelperEvent::Done) => {
                let _ = evt_tx.send(AsrEvent::Done).await;
                return;
            }
            Ok(HelperEvent::Error { message, .. }) => {
                let _ = evt_tx
                    .send(AsrEvent::Error {
                        err: AsrError::Server(message),
                    })
                    .await;
                return;
            }
            Err(e) => {
                let _ = evt_tx
                    .send(AsrEvent::Error {
                        err: AsrError::Protocol(format!("apple helper event: {e}")),
                    })
                    .await;
                return;
            }
        }
    }
}

fn choose_language(cfg: &AppleConfig, ctx: &SessionCtx) -> String {
    if let Some(language) = &cfg.language {
        return language.clone();
    }
    match &ctx.language {
        LanguageMode::Single(language) => language.clone(),
        LanguageMode::Multilingual { hint } => hint
            .iter()
            .find(|s| s.starts_with("zh"))
            .or_else(|| hint.first())
            .cloned()
            .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string()),
    }
}

fn ensure_helper_binary() -> Result<PathBuf, AsrError> {
    let path = helper_cache_path()?;
    let lock_path = path.with_extension("lock");
    ensure_helper_binary_at(&path, &lock_path, HELPER_BYTES)
}

fn ensure_helper_binary_at(
    path: &Path,
    lock_path: &Path,
    helper_bytes: &[u8],
) -> Result<PathBuf, AsrError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AsrError::Server(format!("create helper dir: {e}")))?;
    }

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
        .map_err(|e| AsrError::Server(format!("open helper lock: {e}")))?;
    lock_exclusive(&lock).map_err(|e| AsrError::Server(format!("lock helper: {e}")))?;
    let result = publish_helper_locked(path, helper_bytes);
    let _ = unlock(&lock);
    result
}

fn publish_helper_locked(path: &Path, helper_bytes: &[u8]) -> Result<PathBuf, AsrError> {
    if file_contents_equal(path, helper_bytes)
        .map_err(|e| AsrError::Server(format!("read helper: {e}")))?
    {
        return Ok(path.to_path_buf());
    }

    let tmp_path = path.with_file_name(format!(
        "{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("apple_helper"),
        std::process::id()
    ));
    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| AsrError::Server(format!("write helper: {e}")))?;
    file.write_all(helper_bytes)
        .map_err(|e| AsrError::Server(format!("write helper: {e}")))?;
    file.sync_all()
        .map_err(|e| AsrError::Server(format!("sync helper: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file
            .metadata()
            .map_err(|e| AsrError::Server(format!("stat helper: {e}")))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp_path, perms)
            .map_err(|e| AsrError::Server(format!("chmod helper: {e}")))?;
    }
    drop(file);
    match std::fs::rename(&tmp_path, path) {
        Ok(()) => Ok(path.to_path_buf()),
        Err(error) => {
            let _ = std::fs::remove_file(&tmp_path);
            Err(AsrError::Server(format!("publish helper: {error}")))
        }
    }
}

fn file_contents_equal(path: &Path, expected: &[u8]) -> std::io::Result<bool> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if metadata.len() != expected.len() as u64 {
        return Ok(false);
    }
    let mut body = Vec::with_capacity(expected.len());
    file.read_to_end(&mut body)?;
    Ok(body == expected)
}

fn lock_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn unlock(file: &std::fs::File) -> std::io::Result<()> {
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn helper_cache_path() -> Result<PathBuf, AsrError> {
    let base = if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = std::env::var("HOME")
            .map_err(|_| AsrError::Server("HOME not set for helper cache".into()))?;
        PathBuf::from(home).join(".cache")
    };
    Ok(base.join("shuohua/apple_helper"))
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "event")]
enum HelperEvent {
    #[serde(rename = "partial")]
    Partial { text: String, seq: u64 },
    #[serde(rename = "segment")]
    Segment {
        text: String,
        start_ms: u64,
        end_ms: u64,
    },
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "error")]
    Error {
        message: String,
        #[serde(default)]
        code: Option<String>,
    },
}

fn parse_helper_event(line: &str) -> Result<HelperEvent, serde_json::Error> {
    serde_json::from_str(line)
}

fn encode_pcm_frame(pcm: &[i16], is_last: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 4 + pcm.len() * 2);
    out.push(if is_last { 1 } else { 0 });
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    for &sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_idle_pause_and_finalize_timeout_fields() {
        let cfg: AppleConfig = toml::from_str(
            r#"
idle_pause = true
finalize_timeout_ms = 3000
"#,
        )
        .unwrap();
        assert!(cfg.idle_pause);
        assert_eq!(cfg.finalize_timeout_ms, 3000);

        let default = AppleConfig::default();
        assert!(!default.idle_pause);
        assert_eq!(default.finalize_timeout_ms, 5000);
    }

    #[test]
    fn parse_partial_event_from_helper_json() {
        let event = parse_helper_event(r#"{"event":"partial","text":"测试","seq":7}"#).unwrap();
        match event {
            HelperEvent::Partial { text, seq } => {
                assert_eq!(text, "测试");
                assert_eq!(seq, 7);
            }
            other => panic!("expected partial, got {other:?}"),
        }
    }

    #[test]
    fn macos_version_gate_rejects_before_26_with_cloud_hint() {
        let error = ensure_supported_macos_version(15).unwrap_err();

        assert!(error.to_string().contains("macOS 26"), "{error:#}");
        assert!(error.to_string().contains("cloud ASR"), "{error:#}");
        assert!(ensure_supported_macos_version(26).is_ok());
    }

    #[test]
    fn parses_macos_major_version() {
        assert_eq!(parse_macos_major_version("26.0"), Some(26));
        assert_eq!(parse_macos_major_version("15.7.1\n"), Some(15));
        assert_eq!(parse_macos_major_version(""), None);
    }

    #[test]
    fn parse_segment_event_from_helper_json() {
        let event =
            parse_helper_event(r#"{"event":"segment","text":"完成","start_ms":120,"end_ms":920}"#)
                .unwrap();
        match event {
            HelperEvent::Segment {
                text,
                start_ms,
                end_ms,
            } => {
                assert_eq!(text, "完成");
                assert_eq!(start_ms, 120);
                assert_eq!(end_ms, 920);
            }
            other => panic!("expected segment, got {other:?}"),
        }
    }

    #[test]
    fn encode_pcm_frame_is_flag_count_then_little_endian_samples() {
        let frame = encode_pcm_frame(&[-1, 0, 258], true);
        assert_eq!(frame[0], 1);
        assert_eq!(&frame[1..5], &3u32.to_le_bytes());
        assert_eq!(&frame[5..], &[0xff, 0xff, 0, 0, 2, 1]);
    }

    #[test]
    fn choose_language_prefers_config_then_zh_hint() {
        let ctx = SessionCtx {
            language: LanguageMode::Multilingual {
                hint: vec!["en-US".into(), "zh-CN".into()],
            },
            hotwords: vec![],
        };
        assert_eq!(
            choose_language(
                &AppleConfig {
                    language: Some("en-US".into()),
                    install_assets: true,
                    idle_pause: false,
                    finalize_timeout_ms: default_finalize_timeout_ms(),
                },
                &ctx,
            ),
            "en-US"
        );
        assert_eq!(choose_language(&AppleConfig::default(), &ctx), "zh-CN");
    }

    #[test]
    fn runtime_notice_mentions_possible_asset_install() {
        let provider = AppleProvider {
            config: AppleConfig {
                install_assets: true,
                ..AppleConfig::default()
            },
        };

        let notice = provider.runtime_check_notice().unwrap();
        assert!(notice.line.contains("install speech assets"));
    }

    #[test]
    fn runtime_notice_is_empty_when_asset_install_is_disabled() {
        let provider = AppleProvider {
            config: AppleConfig {
                install_assets: false,
                ..AppleConfig::default()
            },
        };

        assert!(provider.runtime_check_notice().is_none());
    }

    #[test]
    fn helper_publish_skips_rewrite_when_existing_bytes_match() {
        let dir = std::env::temp_dir().join(format!("shuohua-helper-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let helper = dir.join("apple_helper");
        let lock = dir.join("apple_helper.lock");
        std::fs::write(&helper, HELPER_BYTES).unwrap();
        let before = std::fs::metadata(&helper).unwrap().modified().unwrap();

        std::thread::sleep(Duration::from_millis(5));
        let path = ensure_helper_binary_at(&helper, &lock, HELPER_BYTES).unwrap();

        let after = std::fs::metadata(&helper).unwrap().modified().unwrap();
        assert_eq!(path, helper);
        assert_eq!(before, after, "matching helper should not be rewritten");
        let _ = std::fs::remove_dir_all(dir);
    }
}
