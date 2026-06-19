//! UDS client helpers shared by smart fallback and TUI.

use std::path::Path;

use anyhow::{Context, Result};
use std::ops::ControlFlow;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::ipc::protocol::{Command, Event};

pub struct IpcClient {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl IpcClient {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let stream = UnixStream::connect(path)
            .await
            .with_context(|| format!("connect UDS {}", path.display()))?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            lines: BufReader::new(reader).lines(),
            writer,
        })
    }

    pub async fn send(&mut self, command: &Command) -> Result<()> {
        let line = crate::ipc::protocol::encode_command(command)?;
        self.writer
            .write_all(line.as_bytes())
            .await
            .context("write IPC command")
    }

    pub async fn recv(&mut self) -> Result<Option<Event>> {
        let Some(line) = self.lines.next_line().await.context("read IPC event")? else {
            return Ok(None);
        };
        Ok(Some(crate::ipc::protocol::decode_event(&line)?))
    }

    pub async fn recv_until<T>(
        &mut self,
        mut f: impl FnMut(Event) -> Result<ControlFlow<T>>,
    ) -> Result<Option<T>> {
        while let Some(event) = self.recv().await? {
            if let ControlFlow::Break(value) = f(event)? {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }
}
