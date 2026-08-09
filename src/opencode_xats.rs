//! xats control-plane client for OpenCode runtime recovery.

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
const PROTOCOL_VERSION: u32 = 1;
const COMMIT_ATTEMPTS: usize = 3;
const COMMIT_RETRY_DELAY: Duration = Duration::from_millis(200);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PID_FILE_BYTES: u64 = 4 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const RESERVE_PATH: &str = "/api/runtime/opencode/reserve";
const COMMIT_PATH: &str = "/api/runtime/opencode/commit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveStatus {
    Reserved,
    AlreadyReserved,
    NeedRegister,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStatus {
    Committed,
    NeedRegister,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DaemonPidFile {
    pid: u32,
    port: u16,
}

struct SecretString(String);

impl SecretString {
    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("***")
    }
}

#[derive(Debug)]
struct ControlPlane {
    base_url: Url,
    token: Option<SecretString>,
}

enum ControlFailure {
    Retryable(anyhow::Error),
    Fatal(anyhow::Error),
}

impl ControlFailure {
    fn into_error(self) -> anyhow::Error {
        match self {
            Self::Retryable(error) | Self::Fatal(error) => error,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnregisteredOutcome {
    ok: bool,
    need_register: bool,
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReservedOutcome {
    ok: bool,
    state: String,
    runtime_generation: i64,
    changed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommittedOutcome {
    ok: bool,
    state: String,
    delivery_committed: bool,
    connection_bound: bool,
    recovery_prompt: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NeedRegisterOutcome {
    ok: bool,
    error: String,
    need_register: bool,
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialCommitOutcome {
    ok: bool,
    error: String,
    delivery_committed: bool,
    connection_bound: bool,
    detail: PartialCommitDetail,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialCommitDetail {
    error: String,
    /// Accepted so `deny_unknown_fields` still admits the daemon payload.
    #[serde(default)]
    #[allow(dead_code)]
    detail: Option<serde_json::Value>,
    #[serde(default)]
    transport_used: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtocolMismatchOutcome {
    ok: bool,
    error: String,
    cli_protocol_version: u32,
    daemon_protocol_version: u32,
}

enum ParsedCommitOutcome {
    Committed,
    NeedRegister,
    Retryable(anyhow::Error),
    Fatal(anyhow::Error),
}

#[derive(Serialize)]
struct ReserveRequest<'a> {
    identity_key: &'a str,
    runtime_generation: i64,
    protocol_version: u32,
}

#[derive(Serialize)]
struct CommitRequest<'a> {
    identity_key: &'a str,
    runtime_generation: i64,
    protocol_version: u32,
    base_url: &'a str,
    session_id: &'a str,
}

pub fn reserve(identity_key: &str, generation: i64) -> Result<ReserveStatus> {
    validate_identity_key(identity_key)?;
    validate_generation(generation)?;
    let control = discover_control_plane()?;
    reserve_with_control_sync(control, identity_key, generation, CONTROL_TIMEOUT)
}

fn reserve_with_control_sync(
    control: ControlPlane,
    identity_key: &str,
    generation: i64,
    timeout: Duration,
) -> Result<ReserveStatus> {
    let identity_key = identity_key.to_string();
    let worker = std::thread::Builder::new()
        .name("aoe-xats-reserve".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("building xats reserve runtime")?;
            runtime.block_on(reserve_with_control(
                &control,
                &identity_key,
                generation,
                timeout,
            ))
        })
        .context("spawning xats reserve worker")?;
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("xats reserve worker panicked"))?
}

pub async fn commit(
    identity_key: &str,
    generation: i64,
    base_url: &str,
    session_id: &str,
) -> Result<CommitStatus> {
    validate_identity_key(identity_key)?;
    validate_generation(generation)?;
    validate_base_url(base_url)?;
    crate::opencode_runtime::validate_session_id(session_id)?;
    let control = discover_control_plane()?;
    commit_with_control(
        &control,
        identity_key,
        generation,
        base_url,
        session_id,
        COMMIT_ATTEMPTS,
        COMMIT_RETRY_DELAY,
        CONTROL_TIMEOUT,
    )
    .await
}

async fn reserve_with_control(
    control: &ControlPlane,
    identity_key: &str,
    generation: i64,
    timeout: Duration,
) -> Result<ReserveStatus> {
    let result = async {
        let client = build_control_client(timeout)?;
        let request = ReserveRequest {
            identity_key,
            runtime_generation: generation,
            protocol_version: PROTOCOL_VERSION,
        };
        let response = invoke(control, &client, RESERVE_PATH, &request)
            .await
            .map_err(ControlFailure::into_error)
            .context("reserving OpenCode xats runtime")?;
        parse_reserve_outcome(response, generation)
    }
    .await;
    redact_secrets(result, identity_key, control)
}

#[allow(clippy::too_many_arguments)]
async fn commit_with_control(
    control: &ControlPlane,
    identity_key: &str,
    generation: i64,
    base_url: &str,
    session_id: &str,
    attempts: usize,
    retry_delay: Duration,
    timeout: Duration,
) -> Result<CommitStatus> {
    let result = commit_with_control_unredacted(
        control,
        identity_key,
        generation,
        base_url,
        session_id,
        attempts,
        retry_delay,
        timeout,
    )
    .await;
    redact_secrets(result, identity_key, control)
}

#[allow(clippy::too_many_arguments)]
async fn commit_with_control_unredacted(
    control: &ControlPlane,
    identity_key: &str,
    generation: i64,
    base_url: &str,
    session_id: &str,
    attempts: usize,
    retry_delay: Duration,
    timeout: Duration,
) -> Result<CommitStatus> {
    if attempts == 0 {
        bail!("xats commit requires at least one attempt");
    }
    let client = build_control_client(timeout)?;
    let request = CommitRequest {
        identity_key,
        runtime_generation: generation,
        protocol_version: PROTOCOL_VERSION,
        base_url,
        session_id,
    };
    let mut last_error = None;
    for attempt in 1..=attempts {
        match invoke(control, &client, COMMIT_PATH, &request).await {
            Ok(response) => match parse_commit_outcome(response)? {
                ParsedCommitOutcome::Committed => return Ok(CommitStatus::Committed),
                ParsedCommitOutcome::NeedRegister => return Ok(CommitStatus::NeedRegister),
                ParsedCommitOutcome::Fatal(error) => return Err(error),
                ParsedCommitOutcome::Retryable(error) => last_error = Some(error),
            },
            Err(ControlFailure::Fatal(error)) => return Err(error),
            Err(ControlFailure::Retryable(error)) => last_error = Some(error),
        }
        if attempt < attempts {
            tokio::time::sleep(retry_delay).await;
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("xats commit failed")))
        .context("committing OpenCode xats runtime after bounded retry")
}

fn build_control_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .context("building xats HTTP client")
}

async fn invoke<T: Serialize + ?Sized>(
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

fn validate_unregistered(outcome: UnregisteredOutcome) -> Result<()> {
    if !outcome.ok || !outcome.need_register || outcome.state != "unregistered" {
        bail!("xats returned an invalid unregistered outcome");
    }
    Ok(())
}

fn validate_reserved(outcome: &ReservedOutcome, generation: i64) -> Result<()> {
    if !outcome.ok || outcome.state != "reserved" || outcome.runtime_generation != generation {
        bail!("xats returned an invalid reserve outcome");
    }
    Ok(())
}

fn validate_committed(outcome: CommittedOutcome) -> Result<()> {
    if !outcome.ok
        || outcome.state != "delivery_committed"
        || !outcome.delivery_committed
        || outcome.connection_bound
        || outcome.recovery_prompt != "scheduled"
    {
        bail!("xats returned an invalid commit outcome");
    }
    Ok(())
}

fn parse_reserve_outcome(value: serde_json::Value, generation: i64) -> Result<ReserveStatus> {
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        return match value.get("state").and_then(serde_json::Value::as_str) {
            Some("unregistered") => {
                let outcome: UnregisteredOutcome = parse_strict(value, "unregistered")?;
                validate_unregistered(outcome)?;
                Ok(ReserveStatus::NeedRegister)
            }
            Some("reserved") => {
                let outcome: ReservedOutcome = parse_strict(value, "reserve")?;
                validate_reserved(&outcome, generation)?;
                if outcome.changed {
                    Ok(ReserveStatus::Reserved)
                } else {
                    Ok(ReserveStatus::AlreadyReserved)
                }
            }
            _ => bail!("xats returned an invalid reserve outcome"),
        };
    }
    let error = domain_error_code(&value)?.to_string();
    if error == "protocol_version_mismatch" {
        return Err(protocol_mismatch(parse_strict(value, "protocol mismatch")?));
    }
    Err(domain_failure("reserve", &value, &error))
}

fn parse_commit_outcome(value: serde_json::Value) -> Result<ParsedCommitOutcome> {
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        let outcome: CommittedOutcome = parse_strict(value, "commit")?;
        validate_committed(outcome)?;
        return Ok(ParsedCommitOutcome::Committed);
    }
    let error = domain_error_code(&value)?.to_string();
    match error.as_str() {
        "need_register" => {
            let outcome: NeedRegisterOutcome = parse_strict(value, "need-register")?;
            if outcome.ok
                || outcome.error != "need_register"
                || !outcome.need_register
                || outcome.state != "unregistered"
            {
                bail!("xats returned an invalid need-register outcome");
            }
            Ok(ParsedCommitOutcome::NeedRegister)
        }
        "protocol_version_mismatch" => {
            let outcome = parse_strict(value, "protocol mismatch")?;
            Ok(ParsedCommitOutcome::Fatal(protocol_mismatch(outcome)))
        }
        "connection_bind_trigger_failed" => {
            let diagnostic = value.clone();
            let outcome: PartialCommitOutcome = parse_strict(value, "partial commit")?;
            validate_partial_commit(&outcome)?;
            Ok(ParsedCommitOutcome::Retryable(domain_failure(
                "commit",
                &diagnostic,
                &error,
            )))
        }
        "opencode_unreachable" | "session_not_found" => {
            if value.get("detail").is_none() {
                bail!("xats returned an invalid retryable commit outcome");
            }
            Ok(ParsedCommitOutcome::Retryable(domain_failure(
                "commit", &value, &error,
            )))
        }
        _ => Ok(ParsedCommitOutcome::Fatal(domain_failure(
            "commit", &value, &error,
        ))),
    }
}

fn validate_partial_commit(outcome: &PartialCommitOutcome) -> Result<()> {
    if outcome.ok
        || outcome.error != "connection_bind_trigger_failed"
        || !outcome.delivery_committed
        || outcome.connection_bound
        || outcome.detail.error.is_empty()
        || outcome
            .detail
            .transport_used
            .as_deref()
            .is_some_and(str::is_empty)
    {
        bail!("xats returned an invalid partial commit outcome");
    }
    Ok(())
}

fn domain_error_code(value: &serde_json::Value) -> Result<&str> {
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

fn parse_strict<T: for<'de> Deserialize<'de>>(value: serde_json::Value, label: &str) -> Result<T> {
    serde_json::from_value(value)
        .with_context(|| format!("xats protocol mismatch: invalid {label} outcome"))
}

fn protocol_mismatch(outcome: ProtocolMismatchOutcome) -> anyhow::Error {
    if outcome.ok || outcome.error != "protocol_version_mismatch" {
        return anyhow::anyhow!("xats returned an invalid protocol mismatch outcome");
    }
    anyhow::anyhow!(
        "xats protocol mismatch: client {}, daemon {}",
        outcome.cli_protocol_version,
        outcome.daemon_protocol_version
    )
}

fn domain_failure(operation: &str, value: &serde_json::Value, error: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "xats {} returned fail-closed error '{}': {}",
        operation,
        error,
        value
            .get("detail")
            .map_or_else(|| "no detail".to_string(), serde_json::Value::to_string,)
    )
}

fn discover_control_plane() -> Result<ControlPlane> {
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

fn discover_control_plane_in(home: &Path, token: Option<String>) -> Result<ControlPlane> {
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

fn validate_generation(value: i64) -> Result<()> {
    if !(1..=crate::db::MAX_XATS_RUNTIME_GENERATION).contains(&value) {
        bail!("xats runtime generation must be a positive safe integer")
    }
    Ok(())
}

fn validate_base_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value).context("invalid OpenCode base URL")?;
    let loopback = url
        .host_str()
        .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"));
    if url.scheme() != "http" || !loopback || url.port().is_none() {
        bail!("OpenCode base URL must be an explicit loopback HTTP endpoint")
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

fn redact_secrets<T>(result: Result<T>, identity_key: &str, control: &ControlPlane) -> Result<T> {
    result.map_err(|error| {
        let mut diagnostic = format!("{error:#}").replace(identity_key, "***");
        if let Some(token) = &control.token {
            diagnostic = diagnostic.replace(token.expose(), "***");
        }
        anyhow::anyhow!(diagnostic)
    })
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests {
    use super::test_support::{
        committed_response, control_for, partial_commit_response, reserved_response,
        spawn_fake_server, spawn_fake_server_without_content_length, write_pid_file, FakeResponse,
    };
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

    #[tokio::test]
    async fn reserve_uses_exact_path_bearer_and_strict_json_body() {
        let server = spawn_fake_server(vec![FakeResponse {
            status: 200,
            body: reserved_response(7, true),
            delay: Duration::ZERO,
        }]);
        let (_directory, control) = control_for(&server, Some("bearer-token"));
        let status = reserve_with_control(&control, "secret-key", 7, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(status, ReserveStatus::Reserved);
        let request = server.requests.recv().unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /api/runtime/opencode/reserve HTTP/1.1"));
        assert!(lower.contains("authorization: bearer bearer-token"));
        let body = request.split("\r\n\r\n").nth(1).unwrap();
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "identity_key": "secret-key",
                "runtime_generation": 7,
                "protocol_version": 1,
            })
        );
        server.worker.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn synchronous_reserve_is_safe_inside_a_tokio_runtime() {
        let server = spawn_fake_server(vec![FakeResponse {
            status: 200,
            body: reserved_response(7, true),
            delay: Duration::ZERO,
        }]);
        let (_directory, control) = control_for(&server, None);
        let status =
            reserve_with_control_sync(control, "secret-key", 7, Duration::from_secs(1)).unwrap();
        assert_eq!(status, ReserveStatus::Reserved);
        assert!(server.requests.recv().is_ok());
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn reserve_maps_fresh_and_idempotent_domain_outcomes() {
        let server = spawn_fake_server(vec![
            FakeResponse {
                status: 200,
                body: r#"{"ok":true,"need_register":true,"state":"unregistered"}"#.to_string(),
                delay: Duration::ZERO,
            },
            FakeResponse {
                status: 200,
                body: reserved_response(7, false),
                delay: Duration::ZERO,
            },
        ]);
        let (_directory, control) = control_for(&server, None);
        let fresh = reserve_with_control(&control, "secret-key", 7, Duration::from_secs(1))
            .await
            .unwrap();
        let idempotent = reserve_with_control(&control, "secret-key", 7, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(fresh, ReserveStatus::NeedRegister);
        assert_eq!(idempotent, ReserveStatus::AlreadyReserved);
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn transport_status_and_response_errors_never_leak_identity() {
        let secret = "secret-key";
        let token = "secret-bearer-token";
        let server = spawn_fake_server(vec![
            FakeResponse {
                status: 401,
                body: format!(r#"{{"error":"{secret}"}}"#),
                delay: Duration::ZERO,
            },
            FakeResponse {
                status: 200,
                body: format!(r#"{{"ok":false,"error":"stale","detail":"{secret}"}}"#),
                delay: Duration::ZERO,
            },
            FakeResponse {
                status: 200,
                body: format!(
                    r#"{{"ok":true,"state":"reserved","runtime_generation":7,"changed":true,"extra":"{secret}"}}"#
                ),
                delay: Duration::ZERO,
            },
            FakeResponse {
                status: 200,
                body: format!(r#"{{"ok":false,"error":"missing_auth_token","detail":"{token}"}}"#),
                delay: Duration::ZERO,
            },
        ]);
        let (_directory, control) = control_for(&server, Some(token));
        assert!(!format!("{control:?}").contains(token));

        let auth_error = reserve_with_control(&control, secret, 7, Duration::from_secs(1))
            .await
            .unwrap_err();
        let diagnostic = format!("{auth_error:#}");
        assert!(diagnostic.contains("client error HTTP 401"));
        assert!(!diagnostic.contains(secret));

        let domain_error = reserve_with_control(&control, secret, 7, Duration::from_secs(1))
            .await
            .unwrap_err();
        let diagnostic = format!("{domain_error:#}");
        assert!(diagnostic.contains("fail-closed error 'stale'"));
        assert!(!diagnostic.contains(secret));

        let schema_error = reserve_with_control(&control, secret, 7, Duration::from_secs(1))
            .await
            .unwrap_err();
        let diagnostic = format!("{schema_error:#}");
        assert!(diagnostic.contains("protocol mismatch: invalid reserve outcome"));
        assert!(!diagnostic.contains(secret));

        let token_error = reserve_with_control(&control, secret, 7, Duration::from_secs(1))
            .await
            .unwrap_err();
        let diagnostic = format!("{token_error:#}");
        assert!(diagnostic.contains("missing_auth_token"));
        assert!(!diagnostic.contains(token));
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn rest_timeout_is_bounded() {
        let server = spawn_fake_server(vec![FakeResponse {
            status: 200,
            body: reserved_response(7, true),
            delay: Duration::from_millis(200),
        }]);
        let (_directory, control) = control_for(&server, None);
        let started = std::time::Instant::now();
        let error = reserve_with_control(&control, "secret-key", 7, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("sending xats REST request"));
        assert!(started.elapsed() < Duration::from_secs(1));
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn commit_retries_transport_failure_and_sends_exact_runtime_tuple() {
        let server = spawn_fake_server(vec![
            FakeResponse {
                status: 503,
                body: r#"{"ok":false,"error":"storage_unavailable"}"#.to_string(),
                delay: Duration::ZERO,
            },
            FakeResponse {
                status: 200,
                body: committed_response(),
                delay: Duration::ZERO,
            },
        ]);
        let (_directory, control) = control_for(&server, None);
        let status = commit_with_control(
            &control,
            "secret-key",
            9,
            "http://127.0.0.1:8123",
            "ses_left",
            3,
            Duration::ZERO,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(status, CommitStatus::Committed);
        for _ in 0..2 {
            let request = server.requests.recv().unwrap();
            assert!(request.starts_with("POST /api/runtime/opencode/commit HTTP/1.1"));
            let body = request.split("\r\n\r\n").nth(1).unwrap();
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(
                body,
                serde_json::json!({
                    "identity_key": "secret-key",
                    "runtime_generation": 9,
                    "protocol_version": 1,
                    "base_url": "http://127.0.0.1:8123",
                    "session_id": "ses_left",
                })
            );
        }
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn commit_retries_partial_domain_outcome_with_the_exact_tuple() {
        let server = spawn_fake_server(vec![
            FakeResponse {
                status: 200,
                body: partial_commit_response(),
                delay: Duration::ZERO,
            },
            FakeResponse {
                status: 200,
                body: committed_response(),
                delay: Duration::ZERO,
            },
        ]);
        let (_directory, control) = control_for(&server, None);
        let status = commit_with_control(
            &control,
            "secret-key",
            9,
            "http://127.0.0.1:8123",
            "ses_left",
            3,
            Duration::ZERO,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(status, CommitStatus::Committed);
        let first = server.requests.recv().unwrap();
        let second = server.requests.recv().unwrap();
        assert_eq!(
            first.split("\r\n\r\n").nth(1),
            second.split("\r\n\r\n").nth(1)
        );
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn commit_partial_retry_exhaustion_is_bounded() {
        let server = spawn_fake_server(
            (0..3)
                .map(|_| FakeResponse {
                    status: 200,
                    body: partial_commit_response(),
                    delay: Duration::ZERO,
                })
                .collect(),
        );
        let (_directory, control) = control_for(&server, None);
        let error = commit_with_control(
            &control,
            "secret-key",
            9,
            "http://127.0.0.1:8123",
            "ses_left",
            3,
            Duration::ZERO,
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("connection_bind_trigger_failed"));
        for _ in 0..3 {
            assert!(server.requests.recv().is_ok());
        }
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn commit_does_not_retry_client_http_error() {
        let server = spawn_fake_server(vec![FakeResponse {
            status: 401,
            body: r#"{"ok":false,"error":"unauthorized"}"#.to_string(),
            delay: Duration::ZERO,
        }]);
        let (_directory, control) = control_for(&server, None);
        let error = commit_with_control(
            &control,
            "secret-key",
            9,
            "http://127.0.0.1:8123",
            "ses_left",
            3,
            Duration::ZERO,
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("client error HTTP 401"));
        assert!(server.requests.recv().is_ok());
        assert!(server.requests.try_recv().is_err());
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn response_without_content_length_is_read_with_a_hard_limit() {
        let server = spawn_fake_server_without_content_length("x".repeat(MAX_RESPONSE_BYTES + 1));
        let (_directory, control) = control_for(&server, None);
        let error = reserve_with_control(&control, "secret-key", 7, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("response exceeds size limit"));
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn protocol_mismatch_does_not_retry() {
        let server = spawn_fake_server(vec![FakeResponse {
            status: 200,
            body: r#"{"ok":false,"error":"protocol_version_mismatch","cli_protocol_version":2,"daemon_protocol_version":1}"#.to_string(),
            delay: Duration::ZERO,
        }]);
        let (_directory, control) = control_for(&server, None);
        let error = commit_with_control(
            &control,
            "secret-key",
            9,
            "http://127.0.0.1:8123",
            "ses_left",
            3,
            Duration::ZERO,
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("protocol mismatch"));
        assert!(server.requests.recv().is_ok());
        assert!(server.requests.try_recv().is_err());
        server.worker.join().unwrap();
    }

    #[test]
    fn endpoint_validation_is_loopback_only() {
        assert!(validate_base_url("http://127.0.0.1:8123").is_ok());
        assert!(validate_base_url("https://127.0.0.1:8123").is_err());
        assert!(validate_base_url("http://example.com:8123").is_err());
    }
}
