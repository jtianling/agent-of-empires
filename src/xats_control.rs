//! Shared transport for the xats daemon's loopback control plane.
//!
//! Every runtime-control client (OpenCode, kimi) discovers the daemon the same
//! way, speaks the same bearer-authenticated JSON POST, classifies transport
//! failures the same way and must keep the same two secrets out of diagnostics.
//! Those parts live here so a new runtime adds only its endpoint and its
//! outcome schema.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};

pub const IDENTITY_KEY_ENV: &str = "XATS_IDENTITY_KEY";
const XATS_HOME_ENV: &str = "CROSS_AGENT_TEAMS_MCP_HOME";
const XATS_TOKEN_ENV: &str = "CROSS_AGENT_TEAMS_MCP_TOKEN";
pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const MAX_PID_FILE_BYTES: u64 = 4 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DaemonPidFile {
    pid: u32,
    port: u16,
}

pub(crate) struct SecretString(String);

impl SecretString {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("***")
    }
}

#[derive(Debug)]
pub(crate) struct ControlPlane {
    pub(crate) base_url: Url,
    pub(crate) token: Option<SecretString>,
}

pub(crate) enum ControlFailure {
    Retryable(anyhow::Error),
    Fatal(anyhow::Error),
}

impl ControlFailure {
    pub(crate) fn into_error(self) -> anyhow::Error {
        match self {
            Self::Retryable(error) | Self::Fatal(error) => error,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProtocolMismatchOutcome {
    pub(crate) ok: bool,
    pub(crate) error: String,
    pub(crate) cli_protocol_version: u32,
    pub(crate) daemon_protocol_version: u32,
}

pub(crate) fn build_control_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .context("building xats HTTP client")
}

pub(crate) async fn invoke<T: Serialize + ?Sized>(
    control: &ControlPlane,
    client: &reqwest::Client,
    path: &str,
    request: &T,
) -> std::result::Result<serde_json::Value, ControlFailure> {
    let endpoint = control
        .base_url
        .join(path)
        .context("building xats control endpoint")
        .map_err(ControlFailure::Fatal)?;
    let mut builder = client.post(endpoint).json(request);
    if let Some(token) = &control.token {
        builder = builder.bearer_auth(token.expose());
    }
    let mut response = builder
        .send()
        .await
        .context("sending xats REST request")
        .map_err(classify_reqwest_error)?;
    let status = response.status();
    if status != StatusCode::OK {
        let error = anyhow::anyhow!(
            "xats REST returned {} error HTTP {}",
            status_class(status),
            status
        );
        return if status == StatusCode::SERVICE_UNAVAILABLE {
            Err(ControlFailure::Retryable(error))
        } else {
            Err(ControlFailure::Fatal(error))
        };
    }
    if response
        .content_length()
        .is_some_and(|size| size > u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX))
    {
        return Err(ControlFailure::Fatal(anyhow::anyhow!(
            "xats REST response exceeds size limit"
        )));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("reading xats REST response")
        .map_err(ControlFailure::Retryable)?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ControlFailure::Fatal(anyhow::anyhow!(
                "xats REST response exceeds size limit"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .context("xats protocol mismatch: invalid JSON response")
        .map_err(ControlFailure::Fatal)
}

fn classify_reqwest_error(error: anyhow::Error) -> ControlFailure {
    let retryable = error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|error| error.is_timeout() || error.is_connect() || error.is_body());
    if retryable {
        ControlFailure::Retryable(error)
    } else {
        ControlFailure::Fatal(error)
    }
}

pub(crate) fn domain_error_code(value: &serde_json::Value) -> Result<&str> {
    let object = value
        .as_object()
        .context("xats protocol mismatch: outcome is not an object")?;
    if object.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        bail!("xats protocol mismatch: error outcome reports success");
    }
    object
        .get("error")
        .and_then(serde_json::Value::as_str)
        .filter(|error| !error.is_empty())
        .context("xats protocol mismatch: error outcome has no code")
}

pub(crate) fn parse_strict<T: for<'de> Deserialize<'de>>(
    value: serde_json::Value,
    label: &str,
) -> Result<T> {
    serde_json::from_value(value)
        .with_context(|| format!("xats protocol mismatch: invalid {label} outcome"))
}

pub(crate) fn protocol_mismatch(outcome: ProtocolMismatchOutcome) -> anyhow::Error {
    if outcome.ok || outcome.error != "protocol_version_mismatch" {
        return anyhow::anyhow!("xats returned an invalid protocol mismatch outcome");
    }
    anyhow::anyhow!(
        "xats protocol mismatch: client {}, daemon {}",
        outcome.cli_protocol_version,
        outcome.daemon_protocol_version
    )
}

pub(crate) fn domain_failure(
    operation: &str,
    value: &serde_json::Value,
    error: &str,
) -> anyhow::Error {
    anyhow::anyhow!(
        "xats {} returned fail-closed error '{}': {}",
        operation,
        error,
        value
            .get("detail")
            .map_or_else(|| "no detail".to_string(), serde_json::Value::to_string,)
    )
}

pub(crate) fn discover_control_plane() -> Result<ControlPlane> {
    let home = match std::env::var_os(XATS_HOME_ENV) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        Some(_) => bail!("{XATS_HOME_ENV} must not be empty"),
        None => dirs::home_dir()
            .context("finding home directory for xats daemon discovery")?
            .join(".cross-agent-teams-mcp"),
    };
    let token = match std::env::var(XATS_TOKEN_ENV) {
        Ok(value) => Some(validate_token(value)?),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            bail!("{XATS_TOKEN_ENV} must contain valid Unicode")
        }
    };
    discover_control_plane_in(&home, token)
}

pub(crate) fn discover_control_plane_in(
    home: &Path,
    token: Option<String>,
) -> Result<ControlPlane> {
    let pid_path = home.join("daemon.pid");
    let file = std::fs::File::open(&pid_path).with_context(|| {
        format!(
            "reading xats daemon pid file '{}': not available",
            pid_path.display()
        )
    })?;
    let metadata = file
        .metadata()
        .with_context(|| format!("reading xats daemon pid file '{}'", pid_path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_PID_FILE_BYTES {
        bail!("xats daemon pid file is invalid");
    }
    let mut bytes = Vec::new();
    file.take(MAX_PID_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading xats daemon pid file '{}'", pid_path.display()))?;
    if bytes.len() > MAX_PID_FILE_BYTES as usize {
        bail!("xats daemon pid file is invalid");
    }
    let daemon: DaemonPidFile =
        serde_json::from_slice(&bytes).context("xats daemon pid file contains invalid JSON")?;
    validate_daemon_pid(daemon.pid)?;
    if daemon.port == 0 {
        bail!("xats daemon pid file contains invalid port");
    }
    Ok(ControlPlane {
        base_url: Url::parse(&format!("http://127.0.0.1:{}/", daemon.port))
            .context("building xats daemon loopback URL")?,
        token: token.map(SecretString),
    })
}

fn validate_daemon_pid(pid: u32) -> Result<()> {
    if pid <= 1 || pid > i32::MAX as u32 {
        bail!("xats daemon pid file contains invalid pid");
    }
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None) {
        Ok(()) | Err(nix::errno::Errno::EPERM) => Ok(()),
        Err(nix::errno::Errno::ESRCH) => bail!("xats daemon pid is not running"),
        Err(error) => Err(anyhow::Error::new(error).context("checking xats daemon pid")),
    }
}

fn validate_token(value: String) -> Result<String> {
    if value.is_empty()
        || value.len() > 4096
        || value.bytes().any(|byte| matches!(byte, b'\r' | b'\n'))
    {
        bail!("invalid xats bearer token");
    }
    Ok(value)
}

pub(crate) fn validate_identity_key(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_whitespace) {
        bail!("invalid xats identity key")
    }
    Ok(())
}

fn status_class(status: StatusCode) -> &'static str {
    if status.is_client_error() {
        "client"
    } else if status.is_server_error() {
        "server"
    } else {
        "unexpected"
    }
}

/// Strip the two secrets a control-plane diagnostic can carry: the identity key
/// the caller supplied and the bearer token this control plane holds.
pub(crate) fn redact_secrets<T>(
    result: Result<T>,
    identity_key: &str,
    control: &ControlPlane,
) -> Result<T> {
    result.map_err(|error| {
        let mut diagnostic = format!("{error:#}").replace(identity_key, "***");
        if let Some(token) = &control.token {
            diagnostic = diagnostic.replace(token.expose(), "***");
        }
        anyhow::anyhow!(diagnostic)
    })
}

/// Run a control-plane call that has to complete before the caller returns,
/// from a context that may already be inside a Tokio runtime.
pub(crate) fn block_on_control<T, F>(worker_name: &str, task: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>>>> + Send + 'static,
{
    let worker = std::thread::Builder::new()
        .name(worker_name.to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("building xats control runtime")?;
            runtime.block_on(task())
        })
        .context("spawning xats control worker")?;
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("xats control worker panicked"))?
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    use super::test_support::write_pid_file;
    use super::*;

    #[test]
    fn daemon_discovery_validates_pid_file_and_process() {
        let directory = tempfile::tempdir().unwrap();
        let missing = discover_control_plane_in(directory.path(), None).unwrap_err();
        assert!(format!("{missing:#}").contains("not available"));

        std::fs::write(directory.path().join("daemon.pid"), "not-json").unwrap();
        let malformed = discover_control_plane_in(directory.path(), None).unwrap_err();
        assert!(format!("{malformed:#}").contains("invalid JSON"));

        std::fs::write(
            directory.path().join("daemon.pid"),
            vec![b'x'; MAX_PID_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let oversized = discover_control_plane_in(directory.path(), None).unwrap_err();
        assert!(format!("{oversized:#}").contains("pid file is invalid"));

        write_pid_file(directory.path(), 8123, i32::MAX as u32);
        let dead = discover_control_plane_in(directory.path(), None).unwrap_err();
        assert!(format!("{dead:#}").contains("not running"));

        write_pid_file(directory.path(), 8123, std::process::id());
        let control = discover_control_plane_in(directory.path(), None).unwrap();
        assert_eq!(control.base_url.as_str(), "http://127.0.0.1:8123/");
    }
}
