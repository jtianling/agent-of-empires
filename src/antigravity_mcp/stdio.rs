use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;

use anyhow::{Context, Result};
use tokio::io::AsyncWrite;
use tokio::net::unix::pipe::{Receiver, Sender};

use super::{Bridge, IDLE};

pub(super) async fn serve(bridge: &mut Bridge) -> Result<()> {
    let out = File::from(std::io::stdout().as_fd().try_clone_to_owned()?);
    let writer: Box<dyn AsyncWrite + Unpin + Send> = if out.metadata()?.file_type().is_fifo() {
        Box::new(Sender::from_owned_fd(out.into())?)
    } else if out.metadata()?.file_type().is_socket() {
        let socket = UnixStream::from(std::os::fd::OwnedFd::from(out));
        socket.set_nonblocking(true)?;
        Box::new(tokio::net::UnixStream::from_std(socket)?)
    } else {
        Box::new(tokio::io::stdout())
    };
    let stdin = File::from(std::io::stdin().as_fd().try_clone_to_owned()?);
    if stdin.metadata()?.file_type().is_fifo() {
        return bridge
            .serve(Receiver::from_owned_fd(stdin.into())?, writer, IDLE)
            .await;
    }
    // A blocking tty read on Tokio's pool would prevent runtime shutdown after SIGTERM.
    let (mut sink, source) = UnixStream::pair()?;
    source.set_nonblocking(true)?;
    let source = tokio::net::UnixStream::from_std(source)?;
    let (done, result) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("agy-stdin".into())
        .spawn(move || {
            let read = std::io::copy(&mut &stdin, &mut sink);
            let _ = done.send(read);
        })
        .context("Starting Antigravity stdin reader")?;
    bridge.serve(source, writer, IDLE).await?;
    result.await.context("Antigravity stdin reader stopped")??;
    Ok(())
}
