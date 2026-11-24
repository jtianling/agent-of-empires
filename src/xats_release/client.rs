use anyhow::{ensure, Context, Result};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use super::targets::Target;
use crate::xats_control::{self, ControlPlane, PROTOCOL_VERSION};

const RELEASE_PATH: &str = "/api/runtime/panes/release";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    ok: bool,
    protocol_version: u32,
    results: Vec<Outcome>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    index: usize,
    state: String,
    agent: Option<Agent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Agent {
    agent_id: String,
    name: String,
    team: String,
    agent_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    ok: bool,
    protocol_version: u32,
    error: String,
    detail: String,
    target_index: Option<usize>,
    retryable: bool,
}

pub(super) async fn release(
    control: &ControlPlane,
    client: &reqwest::Client,
    targets: &[Target],
) -> Result<()> {
    let mut request = client
        .post(control.base_url.join(RELEASE_PATH)?)
        .json(&json!({
            "protocol_version": PROTOCOL_VERSION, "targets": targets
        }));
    if let Some(token) = &control.token {
        request = request.bearer_auth(token.expose());
    }
    let mut response = request
        .send()
        .await
        .context("Requesting xats pane release")?;
    let status = response.status();
    ensure!(
        status != StatusCode::NOT_FOUND,
        "Xats pane release endpoint is unsupported (HTTP 404): {RELEASE_PATH}"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Reading xats pane release")?
    {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= xats_control::MAX_RESPONSE_BYTES,
            "Xats pane release response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let dead = targets
        .iter()
        .map(|target| target.allow_dead_if_identity_unclaimed)
        .collect::<Vec<_>>();
    decode(status, &bytes, &dead)
}

fn decode(status: StatusCode, bytes: &[u8], dead: &[bool]) -> Result<()> {
    let count = dead.len();
    let value: Value = serde_json::from_slice(bytes)
        .with_context(|| format!("Invalid xats pane release JSON (HTTP {status})"))?;
    if status != StatusCode::OK || value.get("ok") == Some(&Value::Bool(false)) {
        return failure(status, value, count);
    }
    ensure!(
        value["results"].as_array().is_some_and(|results| results
            .iter()
            .all(|result| result.get("agent").is_some()
                && (result["agent"].is_null() || result["agent"].get("agent_type").is_some()))),
        "Xats pane release omitted required nullable fields"
    );
    let response: Response = serde_json::from_value(value)
        .map_err(|_| anyhow::anyhow!("Invalid xats pane release response schema"))?;
    ensure!(
        response.ok && response.protocol_version == PROTOCOL_VERSION,
        "Invalid xats pane release protocol"
    );
    ensure!(
        response.results.len() == count,
        "Xats pane release target count mismatch"
    );
    for (index, result) in response.results.iter().enumerate() {
        ensure!(
            result.index == index,
            "Xats pane release target order mismatch"
        );
        match (&*result.state, &result.agent, dead[index]) {
            ("released", Some(agent), false) => validate_agent(agent)?,
            ("already_unregistered", None, false) => {}
            ("dead_key_unclaimed", None, true) => {}
            _ => anyhow::bail!("Invalid xats pane release state"),
        }
    }
    Ok(())
}

fn validate_agent(agent: &Agent) -> Result<()> {
    uuid::Uuid::parse_str(&agent.agent_id).context("Invalid released xats agent ID")?;
    for value in [&agent.name, &agent.team]
        .into_iter()
        .chain(agent.agent_type.iter())
    {
        ensure!(
            !value.trim().is_empty() && !value.chars().any(char::is_control),
            "Invalid released xats agent metadata"
        );
    }
    Ok(())
}

fn failure(status: StatusCode, value: Value, count: usize) -> Result<()> {
    ensure!(
        value.get("target_index").is_some(),
        "Invalid xats release error (HTTP {status})"
    );
    let failure: Failure = serde_json::from_value(value)
        .map_err(|_| anyhow::anyhow!("Invalid xats pane release error (HTTP {status})"))?;
    ensure!(
        !failure.ok
            && failure.protocol_version == PROTOCOL_VERSION
            && !failure.error.trim().is_empty()
            && failure.target_index.map_or(true, |index| index < count),
        "Invalid xats pane release error protocol (HTTP {status})"
    );
    anyhow::bail!(
        "Xats pane release failed (HTTP {status}, {}, target={:?}, retryable={}): {}",
        failure.error,
        failure.target_index,
        failure.retryable,
        failure.detail
    )
}

#[cfg(test)]
mod tests;
