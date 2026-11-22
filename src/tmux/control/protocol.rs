use std::io::{self, BufReader, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;
use uuid::Uuid;

mod command;
mod frame;

pub(super) use command::native_args;

const TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct Client {
    child: Option<Child>,
    stream: BufReader<Transport>,
    diagnostics: UnixStream,
}

struct Transport {
    socket: UnixStream,
    deadline: Instant,
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(timeout)
}

fn timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "tmux control request timed out")
}

fn socket_error(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::WouldBlock {
        timeout()
    } else {
        error
    }
}

impl Read for Transport {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.socket
            .set_read_timeout(Some(remaining(self.deadline)?))?;
        self.socket.read(out).map_err(socket_error)
    }
}

impl Transport {
    fn send(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            self.socket
                .set_write_timeout(Some(remaining(self.deadline)?))?;
            match self.socket.write(bytes) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(socket_error(e)),
            }
        }
        Ok(())
    }
}

impl Client {
    pub(crate) fn start(mut command: Command) -> anyhow::Result<Self> {
        let (socket, peer) = UnixStream::pair()?;
        command.stdin(Stdio::from(OwnedFd::from(peer.try_clone()?)));
        command.stdout(Stdio::from(OwnedFd::from(peer)));
        let (diagnostics, stderr) = UnixStream::pair()?;
        diagnostics.set_nonblocking(true)?;
        let child = command
            .stderr(Stdio::from(OwnedFd::from(stderr)))
            .spawn()
            .context("Starting tmux control client")?;
        drop(command);
        let mut client = Self {
            child: Some(child),
            diagnostics,
            stream: BufReader::new(Transport {
                socket,
                deadline: Instant::now() + TIMEOUT,
            }),
        };
        let response = client
            .exchange(String::new(), 0)
            .map_err(|error| client.disconnected(error))
            .context("Synchronizing tmux control client")?;
        anyhow::ensure!(
            response.status.success(),
            "tmux control startup failed: {}",
            String::from_utf8_lossy(&response.stderr).trim_end()
        );
        Ok(client)
    }

    pub(crate) fn run(&mut self, args: &[&str]) -> io::Result<Output> {
        if self.child.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "tmux control client disconnected",
            ));
        }
        let request = command::encode(args)?;
        self.exchange(request, 1).map_err(|e| self.disconnected(e))
    }

    fn exchange(&mut self, mut request: String, flags: u8) -> io::Result<Output> {
        let marker = format!("aoe-control-{}", Uuid::new_v4());
        // A separate input line still runs after a tmux command error.
        request.push_str("display-message -p -l ");
        request.push_str(&marker);
        request.push('\n');
        self.stream.get_mut().deadline = Instant::now() + TIMEOUT;
        self.stream.get_mut().send(request.as_bytes())?;
        frame::response(&mut self.stream, flags, marker.as_bytes())
    }

    fn disconnected(&mut self, error: io::Error) -> io::Error {
        self.stop();
        let mut buf = Vec::new();
        let detail = match Read::by_ref(&mut self.diagnostics)
            .take(8192)
            .read_to_end(&mut buf)
        {
            Ok(_) => String::from_utf8_lossy(&buf),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => String::from_utf8_lossy(&buf),
            Err(e) => format!("reading tmux stderr: {e}").into(),
        };
        if detail.is_empty() {
            error
        } else {
            io::Error::new(error.kind(), format!("{error}: {}", detail.trim_end()))
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Err(e) = child.kill() {
                tracing::debug!("Stopping tmux control child {}: {e}", child.id());
            }
            if let Err(e) = child.wait() {
                tracing::warn!("Reaping tmux control child {}: {e}", child.id());
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests;
