//! Apple SpeechAnalyzer provider (macOS 26)。
//!
//! 本文件负责 Rust ↔ Swift helper 的 IPC；真正的 Apple Speech framework 调用和
//! 官方文档链接在 `apple_helper.swift` 顶部。

use crate::asr::types::*;
use crate::config::asr::apple::AppleConfig;
use crate::platform::macos::helper::{encode_pcm_frame, ensure_helper_binary_at};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

const HELPER_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/apple_helper"));
const DEFAULT_LANGUAGE: &str = "zh-CN";

#[cfg(test)]
use crate::config::asr::apple::default_finalize_timeout_ms;

pub struct AppleProvider {
    config: AppleConfig,
}

impl AppleProvider {
    fn from_config(config: AppleConfig) -> anyhow::Result<Self> {
        ensure_supported_macos_version(current_macos_major_version())?;
        Ok(Self { config })
    }

    pub(crate) fn new_from_path_with_overrides(
        path: &std::path::Path,
        overrides: Option<&toml::value::Table>,
    ) -> anyhow::Result<Self> {
        Self::from_config(AppleConfig::from_path_with_overrides(path, overrides)?)
    }

    pub fn finalize_timeout_ms(&self) -> u64 {
        self.config.finalize_timeout_ms
    }

    pub fn options(&self) -> crate::asr::providers::ProviderOptions {
        crate::asr::providers::ProviderOptions {
            local_vad: self.config.local_vad,
            open_timeout_ms: self.config.open_timeout_ms,
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
            .stderr(std::process::Stdio::piped())
            // drop-safe 兜底：session 被 drop 而没走 close() 时，Child drop 即终止
            // helper 子进程；读事件任务随 stdout EOF 自然结束，不留孤儿进程。
            .kill_on_drop(true);

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
                while let Ok(Some(_line)) = lines.next_line().await {}
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
        match tokio::time::timeout(super::SESSION_IO_TIMEOUT, stdin.write_all(&frame)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(AsrError::Network(format!("write apple pcm: {error}"))),
            Err(_) => return Err(AsrError::TransportTimeout),
        }
        if is_last {
            match tokio::time::timeout(super::SESSION_IO_TIMEOUT, stdin.shutdown()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    return Err(AsrError::Network(format!("close apple stdin: {error}")))
                }
                Err(_) => return Err(AsrError::TransportTimeout),
            }
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
        .map_err(|e| AsrError::Server(format!("{e:#}")))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_vad_and_session_timeout_fields() {
        let cfg: AppleConfig = toml::from_str(
            r#"
type = "apple"
local_vad = "on"
open_timeout_ms = 4000
finalize_timeout_ms = 3000
"#,
        )
        .unwrap();
        assert_eq!(cfg.local_vad, crate::config::asr::LocalVadMode::On);
        assert_eq!(cfg.open_timeout_ms, 4000);
        assert_eq!(cfg.finalize_timeout_ms, 3000);

        let default = AppleConfig::default();
        assert_eq!(default.local_vad, crate::config::asr::LocalVadMode::Off);
        assert_eq!(default.open_timeout_ms, 5000);
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
                    _name: None,
                    language: Some("en-US".into()),
                    install_assets: true,
                    local_vad: crate::config::asr::LocalVadMode::Off,
                    open_timeout_ms: 5000,
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

    /// 验证我们依赖的 drop-safe 机制：带 kill_on_drop(true) 的 Command spawn 出的
    /// 子进程，在 Child 被 drop 后会被终止（不留孤儿）。AppleSession 的 helper
    /// 进程靠的就是这条；不依赖真实 Apple helper 二进制，CI 稳定。
    #[tokio::test]
    async fn kill_on_drop_terminates_child_when_dropped_without_close() {
        // /bin/cat 无参数会一直读 stdin，不会自行退出——模拟长驻 helper。
        let child = Command::new("/bin/cat")
            .stdin(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn /bin/cat");
        let pid = child.id().expect("child pid");

        drop(child);

        // drop 触发 kill；等子进程被 reaper 回收。轮询而非固定 sleep，减少抖动。
        let mut alive = true;
        for _ in 0..50 {
            // kill -0：仅探测进程是否存在，不发实际信号。
            let exists = std::process::Command::new("/bin/kill")
                .arg("-0")
                .arg(pid.to_string())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !exists {
                alive = false;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            !alive,
            "child {pid} still alive after drop; kill_on_drop failed"
        );
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
}
