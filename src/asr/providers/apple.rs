//! Apple SpeechAnalyzer provider (macOS 26).

use crate::asr::types::*;
use crate::config::asr::apple::{load_config_with_overrides, AppleConfig};
use async_trait::async_trait;
use serde::Deserialize;
use std::io::Write;
use std::path::PathBuf;
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
        Ok(Self {
            config: load_config_with_overrides(overrides)?,
        })
    }

    pub fn idle_pause(&self) -> bool {
        self.config.idle_pause
    }

    pub fn finalize_timeout_ms(&self) -> u64 {
        self.config.finalize_timeout_ms
    }

    pub async fn check_runtime(&self, ctx: SessionCtx) -> Result<(), AsrError> {
        let (mut session, mut events) = self.open(ctx).await?;
        session.send_pcm(&[], true).await?;
        let done = tokio::time::timeout(Duration::from_millis(self.finalize_timeout_ms()), async {
            while let Some(event) = events.recv().await {
                match event {
                    AsrEvent::Done => return Ok(()),
                    AsrEvent::Error { err } => return Err(err),
                    AsrEvent::Partial { .. } | AsrEvent::Segment { .. } => {}
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
            Ok(HelperEvent::Error { message }) => {
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AsrError::Server(format!("create helper dir: {e}")))?;
    }
    let mut file =
        std::fs::File::create(&path).map_err(|e| AsrError::Server(format!("write helper: {e}")))?;
    file.write_all(HELPER_BYTES)
        .map_err(|e| AsrError::Server(format!("write helper: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file
            .metadata()
            .map_err(|e| AsrError::Server(format!("stat helper: {e}")))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| AsrError::Server(format!("chmod helper: {e}")))?;
    }
    Ok(path)
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
    Error { message: String },
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
}
