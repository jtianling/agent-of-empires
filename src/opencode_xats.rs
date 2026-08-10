//! xats control-plane client for OpenCode runtime recovery.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::xats_control::{
    build_control_client, discover_control_plane, domain_error_code, domain_failure, invoke,
    parse_strict, protocol_mismatch, redact_secrets, ControlFailure, ControlPlane, CONTROL_TIMEOUT,
    PROTOCOL_VERSION,
};

pub(crate) use crate::xats_control::validate_identity_key;

pub use crate::xats_control::IDENTITY_KEY_ENV;

const COMMIT_ATTEMPTS: usize = 3;
const COMMIT_RETRY_DELAY: Duration = Duration::from_millis(200);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xats_control::test_support::{
        control_for, spawn_fake_server, spawn_fake_server_without_content_length, FakeResponse,
    };
    use crate::xats_control::MAX_RESPONSE_BYTES;

    fn reserved_response(generation: i64, changed: bool) -> String {
        format!(
            r#"{{"ok":true,"state":"reserved","runtime_generation":{generation},"changed":{changed}}}"#
        )
    }

    fn committed_response() -> String {
        r#"{"ok":true,"state":"delivery_committed","delivery_committed":true,"connection_bound":false,"recovery_prompt":"scheduled"}"#.to_string()
    }

    fn partial_commit_response() -> String {
        r#"{"ok":false,"error":"connection_bind_trigger_failed","delivery_committed":true,"connection_bound":false,"detail":{"error":"opencode_inject_failed","detail":{"reason":"busy"},"transport_used":"opencode-server"}}"#.to_string()
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
