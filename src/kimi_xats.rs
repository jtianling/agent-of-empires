//! xats control-plane client for kimi delivery coordinates.
//!
//! Kimi has no reserve step and no runtime generation. The shared server offers
//! nothing to fence, so the daemon exposes a single commit that refreshes which
//! session an identity is delivered to, and the absence of a generation from
//! the wire is itself the signal that no fence exists.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::xats_control::{
    block_on_control, build_control_client, discover_control_plane, domain_error_code,
    domain_failure, invoke, parse_strict, protocol_mismatch, redact_secrets, validate_identity_key,
    ControlFailure, ControlPlane, CONTROL_TIMEOUT, PROTOCOL_VERSION,
};

const COMMIT_PATH: &str = "/api/runtime/kimi/commit";
const COMMIT_ATTEMPTS: usize = 3;
const COMMIT_RETRY_DELAY: Duration = Duration::from_millis(200);

/// What the daemon did with this pane's coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStatus {
    Committed {
        changed: bool,
        /// Whether the daemon actually reached the kimi session this call. The
        /// idempotent path reports `false`, and a successful commit that did not
        /// probe says nothing at all about whether the session is alive.
        probed: bool,
    },
    /// No row holds the key and none claims the coordinates. Normal on a pane's
    /// very first launch, before the agent inside it has registered.
    NeedRegister,
}

impl CommitStatus {
    /// Whether this outcome is evidence the live session was verified.
    ///
    /// Deliberately not "did the commit succeed": the idempotent path succeeds
    /// without touching the kimi server, so reading success as health would
    /// report a dead session as connected.
    pub fn session_verified_alive(self) -> bool {
        matches!(self, Self::Committed { probed: true, .. })
    }
}

#[derive(Serialize)]
struct CommitRequest<'a> {
    protocol_version: u32,
    identity_key: &'a str,
    base_url: &'a str,
    session_id: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommittedOutcome {
    ok: bool,
    state: String,
    changed: bool,
    probed: bool,
    agent_id: String,
    /// Read by nothing here; accepted so `deny_unknown_fields` still admits the
    /// daemon's payload rather than reading a complete response as a mismatch.
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    team: String,
    base_url: String,
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NeedRegisterOutcome {
    ok: bool,
    need_register: bool,
    state: String,
    reason: String,
}

enum ParsedCommitOutcome {
    Committed { changed: bool, probed: bool },
    NeedRegister,
    Retryable(anyhow::Error),
    Fatal(anyhow::Error),
}

/// Refresh the delivery coordinates of the identity that owns this pane.
///
/// Run on every launch, not only when the session changed: the conflict check
/// is unconditional, so "another agent row already claims this session" surfaces
/// before the TUI starts instead of as an identity that never came back.
///
/// `previous_session_id` carries the coordinates the row is expected to still
/// hold. A row the agent registered on an earlier launch carries no identity
/// key, because the key never reaches a kimi pane; committing the old
/// coordinates first lets the daemon resolve that row by `(base_url,
/// session_id)` and adopt the key, so the refresh can resolve it by key. Once
/// adopted that first call is a plain idempotent lookup.
pub fn commit(
    identity_key: &str,
    base_url: &str,
    previous_session_id: Option<&str>,
    session_id: &str,
) -> Result<CommitStatus> {
    validate_identity_key(identity_key)?;
    validate_base_url(base_url)?;
    crate::kimi::validate_session_id(session_id)?;
    if let Some(previous) = previous_session_id {
        crate::kimi::validate_session_id(previous)?;
    }
    let control = discover_control_plane()?;
    let identity_key = identity_key.to_string();
    let base_url = base_url.to_string();
    let previous_session_id = previous_session_id.map(str::to_string);
    let session_id = session_id.to_string();
    block_on_control("aoe-kimi-commit", move || {
        Box::pin(async move {
            commit_pair_with_control(
                &control,
                &identity_key,
                &base_url,
                previous_session_id.as_deref(),
                &session_id,
                COMMIT_ATTEMPTS,
                COMMIT_RETRY_DELAY,
                CONTROL_TIMEOUT,
            )
            .await
        })
    })
}

#[allow(clippy::too_many_arguments)]
async fn commit_pair_with_control(
    control: &ControlPlane,
    identity_key: &str,
    base_url: &str,
    previous_session_id: Option<&str>,
    session_id: &str,
    attempts: usize,
    retry_delay: Duration,
    timeout: Duration,
) -> Result<CommitStatus> {
    if let Some(previous) = previous_session_id.filter(|previous| *previous != session_id) {
        commit_with_control(
            control,
            identity_key,
            base_url,
            previous,
            attempts,
            retry_delay,
            timeout,
        )
        .await
        .context("adopting the identity key onto the previous kimi coordinates")?;
    }
    commit_with_control(
        control,
        identity_key,
        base_url,
        session_id,
        attempts,
        retry_delay,
        timeout,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn commit_with_control(
    control: &ControlPlane,
    identity_key: &str,
    base_url: &str,
    session_id: &str,
    attempts: usize,
    retry_delay: Duration,
    timeout: Duration,
) -> Result<CommitStatus> {
    let result = commit_unredacted(
        control,
        identity_key,
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
async fn commit_unredacted(
    control: &ControlPlane,
    identity_key: &str,
    base_url: &str,
    session_id: &str,
    attempts: usize,
    retry_delay: Duration,
    timeout: Duration,
) -> Result<CommitStatus> {
    if attempts == 0 {
        bail!("xats kimi commit requires at least one attempt");
    }
    let client = build_control_client(timeout)?;
    let request = CommitRequest {
        protocol_version: PROTOCOL_VERSION,
        identity_key,
        base_url,
        session_id,
    };
    let mut last_error = None;
    for attempt in 1..=attempts {
        match invoke(control, &client, COMMIT_PATH, &request).await {
            Ok(response) => match parse_commit_outcome(response)? {
                ParsedCommitOutcome::Committed { changed, probed } => {
                    return Ok(CommitStatus::Committed { changed, probed })
                }
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
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("xats kimi commit failed")))
        .context("committing kimi xats delivery after bounded retry")
}

/// Only a refused probe is worth another identical attempt. Every other
/// outcome describes a state a retry cannot change, and `session_claimed_by_\
/// other_agent` in particular must reach the caller before a pane starts.
fn parse_commit_outcome(value: serde_json::Value) -> Result<ParsedCommitOutcome> {
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        if value.get("need_register").is_some() {
            let outcome: NeedRegisterOutcome = parse_strict(value, "kimi need-register")?;
            validate_need_register(&outcome)?;
            return Ok(ParsedCommitOutcome::NeedRegister);
        }
        let outcome: CommittedOutcome = parse_strict(value, "kimi commit")?;
        validate_committed(&outcome)?;
        return Ok(ParsedCommitOutcome::Committed {
            changed: outcome.changed,
            probed: outcome.probed,
        });
    }
    let error = domain_error_code(&value)?.to_string();
    match error.as_str() {
        "protocol_version_mismatch" => {
            let outcome = parse_strict(value, "protocol mismatch")?;
            Ok(ParsedCommitOutcome::Fatal(protocol_mismatch(outcome)))
        }
        "session_not_found" => Ok(ParsedCommitOutcome::Retryable(domain_failure(
            "kimi commit",
            &value,
            &error,
        ))),
        _ => Ok(ParsedCommitOutcome::Fatal(domain_failure(
            "kimi commit",
            &value,
            &error,
        ))),
    }
}

/// `changed` and `probed` are read, never cross-checked: whether the daemon
/// reaches the session on a commit that moved nothing is its own decision, and
/// treating one combination as malformed would fail a launch on an outcome the
/// caller is required to accept as success.
fn validate_committed(outcome: &CommittedOutcome) -> Result<()> {
    if !outcome.ok
        || outcome.state != "committed"
        || outcome.agent_id.is_empty()
        || outcome.base_url.is_empty()
        || outcome.session_id.is_empty()
    {
        bail!("xats returned an invalid kimi commit outcome");
    }
    Ok(())
}

fn validate_need_register(outcome: &NeedRegisterOutcome) -> Result<()> {
    if !outcome.ok
        || !outcome.need_register
        || outcome.state != "unregistered"
        || outcome.reason.is_empty()
    {
        bail!("xats returned an invalid kimi need-register outcome");
    }
    Ok(())
}

fn validate_base_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value).context("invalid kimi base URL")?;
    let loopback = url
        .host_str()
        .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"));
    if url.scheme() != "http" || !loopback || url.port().is_none() {
        bail!("kimi base URL must be an explicit loopback HTTP endpoint")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xats_control::test_support::{control_for, spawn_fake_server, FakeResponse};

    const BASE_URL: &str = "http://127.0.0.1:58627";
    const SESSION: &str = "session_e96b6682";

    fn committed(changed: bool, probed: bool) -> String {
        format!(
            r#"{{"ok":true,"state":"committed","changed":{changed},"probed":{probed},
                 "agent_id":"a1","name":"kimi-1","team":"default",
                 "base_url":"{BASE_URL}","session_id":"{SESSION}"}}"#
        )
    }

    fn reply(status: u16, body: String) -> FakeResponse {
        FakeResponse {
            status,
            body,
            delay: Duration::ZERO,
        }
    }

    async fn commit_once(control: &ControlPlane, key: &str) -> Result<CommitStatus> {
        commit_with_control(
            control,
            key,
            BASE_URL,
            SESSION,
            3,
            Duration::ZERO,
            Duration::from_secs(1),
        )
        .await
    }

    #[tokio::test]
    async fn commit_posts_the_exact_tuple_without_a_runtime_generation() {
        let server = spawn_fake_server(vec![reply(200, committed(true, true))]);
        let (_directory, control) = control_for(&server, Some("bearer-token"));
        let status = commit_once(&control, "secret-key").await.unwrap();
        assert_eq!(
            status,
            CommitStatus::Committed {
                changed: true,
                probed: true
            }
        );
        let request = server.requests.recv().unwrap();
        assert!(request.starts_with("POST /api/runtime/kimi/commit HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer bearer-token"));
        let body: serde_json::Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "protocol_version": 1,
                "identity_key": "secret-key",
                "base_url": BASE_URL,
                "session_id": SESSION,
            })
        );
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn an_idempotent_commit_succeeds_without_claiming_the_session_is_alive() {
        let server = spawn_fake_server(vec![reply(200, committed(false, false))]);
        let (_directory, control) = control_for(&server, None);
        let status = commit_once(&control, "secret-key").await.unwrap();
        assert_eq!(
            status,
            CommitStatus::Committed {
                changed: false,
                probed: false
            }
        );
        assert!(!status.session_verified_alive());
        assert!(CommitStatus::Committed {
            changed: true,
            probed: true
        }
        .session_verified_alive());
        server.worker.join().unwrap();
    }

    /// A resume whose coordinates already matched is still a success, whether or
    /// not the daemon reached the session while confirming it.
    #[tokio::test]
    async fn an_unchanged_commit_is_accepted_even_when_the_daemon_probed() {
        let server = spawn_fake_server(vec![reply(200, committed(false, true))]);
        let (_directory, control) = control_for(&server, None);
        assert_eq!(
            commit_once(&control, "secret-key").await.unwrap(),
            CommitStatus::Committed {
                changed: false,
                probed: true
            }
        );
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn an_unregistered_identity_is_a_normal_first_launch() {
        let server = spawn_fake_server(vec![reply(
            200,
            r#"{"ok":true,"need_register":true,"state":"unregistered",
                "reason":"identity_key_not_found"}"#
                .to_string(),
        )]);
        let (_directory, control) = control_for(&server, None);
        assert_eq!(
            commit_once(&control, "secret-key").await.unwrap(),
            CommitStatus::NeedRegister
        );
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn only_a_refused_probe_is_retried_with_the_identical_tuple() {
        let server = spawn_fake_server(vec![
            reply(
                200,
                r#"{"ok":false,"error":"session_not_found","detail":{"session_id":"x"}}"#
                    .to_string(),
            ),
            reply(200, committed(true, true)),
        ]);
        let (_directory, control) = control_for(&server, None);
        assert!(commit_once(&control, "secret-key").await.is_ok());
        let first = server.requests.recv().unwrap();
        let second = server.requests.recv().unwrap();
        assert_eq!(
            first.split("\r\n\r\n").nth(1),
            second.split("\r\n\r\n").nth(1)
        );
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn a_session_claimed_by_another_agent_fails_closed_on_the_first_reply() {
        let server = spawn_fake_server(vec![reply(
            200,
            r#"{"ok":false,"error":"session_claimed_by_other_agent",
                "conflicting_agent_id":"a2","name":"kimi-2","team":"default"}"#
                .to_string(),
        )]);
        let (_directory, control) = control_for(&server, None);
        let error = commit_once(&control, "secret-key").await.unwrap_err();
        assert!(format!("{error:#}").contains("session_claimed_by_other_agent"));
        assert!(server.requests.recv().is_ok());
        assert!(server.requests.try_recv().is_err());
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn every_other_fail_closed_outcome_is_reported_on_the_first_reply() {
        let bodies = [
            r#"{"ok":false,"error":"protocol_version_mismatch","cli_protocol_version":1,
                "daemon_protocol_version":2}"#,
            r#"{"ok":false,"error":"agent_type_conflict","expected":"kimi-code","actual":"codex"}"#,
            r#"{"ok":false,"error":"missing_auth_token"}"#,
            r#"{"ok":false,"error":"some_future_outcome"}"#,
        ];
        let server = spawn_fake_server(
            bodies
                .iter()
                .map(|body| reply(200, (*body).to_string()))
                .collect(),
        );
        let (_directory, control) = control_for(&server, None);
        for _ in bodies {
            assert!(commit_once(&control, "secret-key").await.is_err());
            assert!(server.requests.recv().is_ok());
        }
        assert!(server.requests.try_recv().is_err());
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn neither_the_identity_key_nor_the_bearer_token_reaches_a_diagnostic() {
        let secret = "secret-key";
        let token = "secret-bearer-token";
        let server = spawn_fake_server(vec![
            reply(401, format!(r#"{{"error":"{secret}"}}"#)),
            reply(
                200,
                format!(r#"{{"ok":false,"error":"missing_auth_token","detail":"{token}"}}"#),
            ),
            reply(
                200,
                format!(r#"{{"ok":true,"state":"committed","extra":"{secret}"}}"#),
            ),
        ]);
        let (_directory, control) = control_for(&server, Some(token));
        for _ in 0..3 {
            let diagnostic = format!("{:#}", commit_once(&control, secret).await.unwrap_err());
            assert!(!diagnostic.contains(secret), "{diagnostic}");
            assert!(!diagnostic.contains(token), "{diagnostic}");
        }
        server.worker.join().unwrap();
    }

    #[tokio::test]
    async fn a_slow_daemon_is_bounded_by_the_request_timeout() {
        let server = spawn_fake_server(vec![FakeResponse {
            status: 200,
            body: committed(true, true),
            delay: Duration::from_millis(200),
        }]);
        let (_directory, control) = control_for(&server, None);
        let started = std::time::Instant::now();
        let error = commit_with_control(
            &control,
            "secret-key",
            BASE_URL,
            SESSION,
            1,
            Duration::ZERO,
            Duration::from_millis(20),
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("sending xats REST request"));
        assert!(started.elapsed() < Duration::from_secs(1));
        server.worker.join().unwrap();
    }

    #[test]
    fn commit_input_is_validated_before_any_daemon_lookup() {
        assert!(validate_base_url("http://127.0.0.1:58627").is_ok());
        assert!(validate_base_url("https://127.0.0.1:58627").is_err());
        assert!(validate_base_url("http://kimi.example.com:58627").is_err());
        assert!(commit("", BASE_URL, None, SESSION).is_err());
        assert!(commit("key", BASE_URL, None, "not-a-session").is_err());
        assert!(commit("key", BASE_URL, Some("not-a-session"), SESSION).is_err());
    }

    /// A row the agent registered on an earlier launch carries no identity key,
    /// so a fresh conversation must let the daemon adopt the key by the old
    /// coordinates before the new ones can be resolved by key.
    #[tokio::test]
    async fn commit_adopts_the_previous_coordinates_before_refreshing() {
        const PREVIOUS: &str = "session_0f0f0f0f";
        let server = spawn_fake_server(vec![
            reply(200, committed(false, false)),
            reply(200, committed(true, true)),
        ]);
        let (_directory, control) = control_for(&server, None);

        commit_pair_with_control(
            &control,
            "secret-key",
            BASE_URL,
            Some(PREVIOUS),
            SESSION,
            3,
            Duration::ZERO,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

        let adopt = server.requests.recv().unwrap();
        let refresh = server.requests.recv().unwrap();
        assert!(adopt.contains(PREVIOUS));
        assert!(!adopt.contains(SESSION));
        assert!(refresh.contains(SESSION));
        assert!(!refresh.contains(PREVIOUS));
    }

    /// Resume keeps the same coordinates, so the adoption call would be a
    /// duplicate of the refresh.
    #[tokio::test]
    async fn commit_skips_adoption_when_the_session_is_unchanged() {
        let server = spawn_fake_server(vec![reply(200, committed(false, false))]);
        let (_directory, control) = control_for(&server, None);

        commit_pair_with_control(
            &control,
            "secret-key",
            BASE_URL,
            Some(SESSION),
            SESSION,
            3,
            Duration::ZERO,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

        assert!(server.requests.recv().unwrap().contains(SESSION));
        assert!(server.requests.try_recv().is_err());
    }
}
