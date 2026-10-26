use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

use crate::xats_control::{self, ControlPlane};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaneTarget {
    pub socket_path: String,
    pub server_pid: u32,
    pub server_started_at: u64,
    pub pane_id: String,
}

impl PaneTarget {
    fn validate(&self) -> Result<()> {
        ensure!(
            Path::new(&self.socket_path).is_absolute()
                && !self.socket_path.chars().any(char::is_control),
            "Invalid xats pane socket path"
        );
        ensure!(
            self.server_pid > 1 && self.server_pid <= i32::MAX as u32,
            "Invalid xats tmux server PID"
        );
        ensure!(
            self.server_started_at > 0 && self.server_started_at <= 9_007_199_254_740_991,
            "Invalid xats tmux server start time"
        );
        let digits = self.pane_id.strip_prefix('%').unwrap_or_default();
        ensure!(
            !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
            "Invalid xats pane ID"
        );
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Agent {
    agent_id: String,
    device: String,
    name: String,
    team: String,
    agent_type: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    status: String,
    agent: Option<Agent>,
    reason: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Runtime {
    status: String,
    reason: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Response {
    ok: bool,
    protocol_version: u32,
    tmux: PaneTarget,
    checked_at: String,
    identity: Identity,
    runtime: Runtime,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    ok: bool,
    error: String,
    detail: String,
}

pub(crate) async fn lookup(target: &PaneTarget) -> Result<Value> {
    target.validate()?;
    let control = xats_control::discover_control_plane()?;
    let result = invoke(&control, target).await;
    let result = result.map_err(|error| {
        let detail = format!("{error:#}");
        let detail = control.token.as_ref().map_or_else(
            || detail.clone(),
            |token| detail.replace(token.expose(), "***"),
        );
        anyhow::anyhow!(detail)
    });
    if let Err(error) = &result {
        tracing::warn!("Xats pane lookup failed: {error:#}");
    }
    result
}

async fn invoke(control: &ControlPlane, target: &PaneTarget) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(xats_control::CONTROL_TIMEOUT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("Building xats pane lookup client")?;
    let endpoint = control.base_url.join("/api/runtime/pane/lookup")?;
    let mut request = client
        .post(endpoint)
        .json(&json!({"protocol_version": 1, "tmux": target}));
    if let Some(token) = &control.token {
        request = request.bearer_auth(token.expose());
    }
    let mut response = request
        .send()
        .await
        .context("Requesting xats pane lookup")?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!(
            "Xats pane lookup endpoint is unsupported (HTTP 404): /api/runtime/pane/lookup"
        );
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("Reading xats pane lookup")? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= xats_control::MAX_RESPONSE_BYTES,
            "Xats pane lookup response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("Xats pane lookup invalid JSON (HTTP {status})"))?;
    if !status.is_success() || value.get("ok") == Some(&Value::Bool(false)) {
        let failure: Failure = serde_json::from_value(value)
            .map_err(|_| anyhow::anyhow!("Xats pane lookup invalid error (HTTP {status})"))?;
        ensure!(
            !failure.ok && !failure.error.is_empty(),
            "Invalid xats lookup error"
        );
        anyhow::bail!(
            "Xats pane lookup failed (HTTP {status}, {}): {}",
            failure.error,
            failure.detail
        );
    }
    validate_response(value, target)
}

fn validate_response(value: Value, target: &PaneTarget) -> Result<Value> {
    ensure!(
        value.pointer("/identity/agent").is_some(),
        "Xats pane lookup omitted nullable agent"
    );
    let response: Response = serde_json::from_value(value)
        .map_err(|_| anyhow::anyhow!("Invalid xats pane lookup response schema"))?;
    ensure!(
        response.ok && response.protocol_version == 1,
        "Invalid xats lookup version"
    );
    ensure!(response.tmux == *target, "Xats pane lookup target mismatch");
    chrono::DateTime::parse_from_rfc3339(&response.checked_at)
        .context("Invalid xats lookup observation time")?;
    ensure!(
        matches!(
            response.identity.status.as_str(),
            "verified" | "unbound" | "unverifiable" | "ambiguous" | "stale"
        ),
        "Invalid xats identity status"
    );
    ensure!(
        (response.identity.status == "verified") == response.identity.agent.is_some(),
        "Xats identity verification and agent disagree"
    );
    ensure!(
        matches!(
            response.runtime.status.as_str(),
            "ready" | "not_ready" | "unknown"
        ) && (response.runtime.status != "ready" || response.identity.status == "verified"),
        "Invalid xats runtime readiness"
    );
    if let Some(agent) = &response.identity.agent {
        uuid::Uuid::try_parse(&agent.agent_id).context("Invalid xats agent ID")?;
        for value in [&agent.device, &agent.name, &agent.team, &agent.agent_type] {
            ensure!(
                !value.trim().is_empty() && !value.chars().any(char::is_control),
                "Invalid xats agent metadata"
            );
        }
    }
    Ok(serde_json::to_value(response)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> PaneTarget {
        PaneTarget {
            socket_path: "/tmp/private-tmux/socket".into(),
            server_pid: 100,
            server_started_at: 1_793_040_000,
            pane_id: "%1".into(),
        }
    }

    fn response() -> Value {
        json!({
            "ok": true, "protocol_version": 1, "tmux": target(),
            "checked_at": "2026-10-27T00:00:00Z",
            "identity": {"status": "verified", "reason": "binding_verified",
                "agent": {"agent_id": "e559f096-96ae-4f9e-9d9c-1908c098bd41",
                    "device": "local", "name": "worker", "team": "project",
                    "agent_type": "codex"}},
            "runtime": {"status": "unknown", "reason": "not_probed"}
        })
    }

    #[test]
    fn accepts_verified_identity_without_claiming_readiness() {
        let result = validate_response(response(), &target()).unwrap();
        assert_eq!(result["identity"]["agent"]["name"], "worker");
        assert_eq!(result["runtime"]["status"], "unknown");
    }

    #[test]
    fn rejects_other_server_or_pane_and_unverified_agent() {
        for (field, replacement) in [
            ("socket_path", json!("/tmp/other")),
            ("server_pid", json!(101)),
            ("server_started_at", json!(1)),
            ("pane_id", json!("%2")),
        ] {
            let mut value = response();
            value["tmux"][field] = replacement;
            assert!(validate_response(value, &target()).is_err());
        }
        let mut value = response();
        value["identity"]["status"] = json!("unverifiable");
        assert!(validate_response(value, &target()).is_err());
    }

    #[test]
    fn rejects_credentials_and_ready_without_verified_identity() {
        let mut value = response();
        value["identity"]["agent"]["token"] = json!("sensitive-value");
        let error = validate_response(value, &target()).unwrap_err();
        assert!(!error.to_string().contains("sensitive-value"));
        let mut value = response();
        value["identity"] = json!({
            "status": "unbound", "agent": null, "reason": "no_binding"});
        value["runtime"]["status"] = json!("ready");
        assert!(validate_response(value, &target()).is_err());
    }
}
