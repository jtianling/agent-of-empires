use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, ensure, Context, Result};
use nix::libc;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt};

use super::{lease, validate_key};
use crate::agents::{antigravity::bridge_key, executable};
use crate::xats_control::{block_on_control, CONTROL_TIMEOUT};

#[cfg(test)]
#[path = "tests/deferred.rs"]
mod tests;

const PROGRAM: &str = "xats-agy-mcp";
const IDENTITY: &str = "XATS_AGY_IDENTITY_KEY";

pub(super) fn requested(marker: Option<&OsStr>) -> Result<bool> {
    match marker {
        None => Ok(false),
        Some(value) if value == "0" => Ok(false),
        Some(value) if value == "1" => Ok(true),
        _ => bail!("Invalid Antigravity deferred launch marker"),
    }
}

pub(crate) fn require_deferred_bridge() -> Result<PathBuf> {
    let path = executable::resolve(PROGRAM).context("Cannot find executable xats-agy-mcp")?;
    let path = path
        .canonicalize()
        .context("Resolving xats-agy-mcp executable")?;
    require_deferred_capability(&path)?;
    Ok(path)
}

pub(crate) fn require_deferred_capability(path: &Path) -> Result<()> {
    probe_with_path(path, &std::env::var_os("PATH").unwrap_or_default())
}

fn probe_with_path(path: &Path, search_path: &OsStr) -> Result<()> {
    let mut command = tokio::process::Command::new(path);
    command
        .arg("--capabilities-json")
        .env_clear()
        .env("PATH", search_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    block_on_control("agy-deferred-capability", move || Box::pin(probe(command)))
        .with_context(|| format!("Checking {} --capabilities-json", path.display()))
}

async fn probe(mut command: tokio::process::Command) -> Result<()> {
    #[derive(Deserialize)]
    struct Capabilities {
        defer_registration: u64,
    }

    let mut child = command.spawn().context("Starting capability probe")?;
    let stdout = child.stdout.take().context("Missing probe stdout")?;
    let stderr = child.stderr.take().context("Missing probe stderr")?;
    let read = async {
        tokio::try_join!(probe_output(stdout), probe_output(stderr), async {
            child.wait().await.context("Waiting for capability probe")
        })
    };
    let result = tokio::time::timeout(CONTROL_TIMEOUT, read)
        .await
        .context("Deferred capability probe timed out")
        .and_then(|r| r);
    let (stdout, stderr, status) = match result {
        Ok(output) => output,
        Err(error) => {
            child
                .kill()
                .await
                .with_context(|| format!("Stopping failed probe: {error:#}"))?;
            return Err(error);
        }
    };
    ensure!(
        status.success(),
        "Capability probe failed ({status}): {}",
        String::from_utf8_lossy(&stderr).trim()
    );
    ensure!(
        stdout.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{'),
        "Invalid deferred capability response: expected a JSON object"
    );
    let caps: Capabilities =
        serde_json::from_slice(&stdout).context("Invalid deferred capability response")?;
    ensure!(
        caps.defer_registration == 1,
        "Upgrade xats: expected defer_registration capability 1"
    );
    Ok(())
}

async fn probe_output(pipe: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    const MAX: u64 = 16 * 1024;
    let mut bytes = Vec::new();
    pipe.take(MAX + 1)
        .read_to_end(&mut bytes)
        .await
        .context("Reading capability probe output")?;
    ensure!(
        bytes.len() <= MAX as usize,
        "Capability probe output exceeds size limit"
    );
    Ok(bytes)
}

fn validate_launch(get: impl Fn(&str) -> Option<OsString>) -> Result<()> {
    ensure!(
        get("AOE_ANTIGRAVITY_NAMED").as_deref() == Some(OsStr::new("1")),
        "Expected wrapper-managed Antigravity launch"
    );
    ensure!(
        get("XATS_AGY_DEFER_REGISTRATION").as_deref() == Some(OsStr::new("1")),
        "Expected XATS_AGY_DEFER_REGISTRATION=1"
    );
    for field in [
        "XATS_AGY_NAME",
        "XATS_AGY_TEAM",
        "XATS_IDENTITY_KEY",
        IDENTITY,
    ] {
        ensure!(
            get(field).is_none(),
            "Unexpected {field} in deferred launch"
        );
    }
    let launch = get("XATS_AGY_LAUNCH_ID").context("Missing XATS_AGY_LAUNCH_ID")?;
    validate_key(launch.to_str().context("Invalid XATS_AGY_LAUNCH_ID")?)
        .context("Invalid XATS_AGY_LAUNCH_ID")?;
    let raw = get("XATS_AGY_PID").context("Missing XATS_AGY_PID")?;
    let pid: i32 = raw
        .to_str()
        .context("Invalid XATS_AGY_PID")?
        .parse()
        .context("Invalid XATS_AGY_PID")?;
    ensure!(
        pid > 1 && raw == pid.to_string().as_str(),
        "Invalid XATS_AGY_PID"
    );
    Ok(())
}

pub(super) fn run() -> Result<()> {
    validate_launch(|field| std::env::var_os(field))?;
    let key = bridge_key()?.context("Deferred Antigravity does not own an xats slot")?;
    validate_key(&key)?;
    let lock = lease::acquire(&crate::session::get_app_dir()?, &key)?;
    let cwd = std::env::current_dir()?;
    crate::hooks::antigravity_xats::check_named_mcp(
        cwd.to_str()
            .context("Invalid Antigravity working directory")?,
    )?;
    let bridge = PreparedBridge::from_env()?;
    ensure!(
        bridge_key()?.as_ref() == Some(&key),
        "Deferred Antigravity no longer owns its slot"
    );
    bridge.exec(&key, lock)
}

struct PreparedBridge {
    path: PathBuf,
    search_path: OsString,
    identity: (u64, u64),
}

impl PreparedBridge {
    fn from_env() -> Result<Self> {
        let path = std::env::var_os("AOE_ANTIGRAVITY_MCP_BINARY")
            .context("Missing AOE_ANTIGRAVITY_MCP_BINARY")?;
        let search_path = std::env::var_os("AOE_ANTIGRAVITY_MCP_PATH")
            .context("Missing AOE_ANTIGRAVITY_MCP_PATH")?;
        Self::check(path.into(), search_path)
    }

    fn check(path: PathBuf, search_path: OsString) -> Result<Self> {
        let identity = file_identity(&path)?;
        probe_with_path(&path, &search_path)?;
        Ok(Self {
            path,
            search_path,
            identity,
        })
    }

    fn exec(self, key: &str, lock: File) -> Result<()> {
        ensure!(
            file_identity(&self.path)? == self.identity,
            "Antigravity MCP executable changed during capability verification"
        );
        let mut command = Command::new(&self.path);
        command.env("PATH", self.search_path).env(IDENTITY, key);
        // Only the final exec inherits the flock. All probes ran with FD_CLOEXEC set.
        let fd = lock.as_raw_fd();
        if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } == -1 {
            return Err(std::io::Error::last_os_error()).context("Retaining Antigravity pane lock");
        }
        Err(command.exec()).with_context(|| format!("Executing {}", self.path.display()))
    }
}

fn file_identity(path: &Path) -> Result<(u64, u64)> {
    ensure!(
        path.is_absolute(),
        "Expected absolute Antigravity MCP executable path"
    );
    let canonical = path
        .canonicalize()
        .with_context(|| format!("Resolving {}", path.display()))?;
    ensure!(
        canonical.as_os_str() == path.as_os_str(),
        "Expected canonical Antigravity MCP executable path"
    );
    let metadata = path
        .metadata()
        .with_context(|| format!("Inspecting {}", path.display()))?;
    ensure!(
        metadata.is_file(),
        "Expected regular Antigravity MCP executable file"
    );
    Ok((metadata.dev(), metadata.ino()))
}
