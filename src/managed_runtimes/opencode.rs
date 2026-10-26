use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::session::{Instance, Storage};

const SERVER_NAME: &str = "aoe-managed-xats";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(crate) fn load(
    profile: &str,
    id: &str,
    slot: i64,
    generation: i64,
    launch_id: Option<&str>,
    pane_id: Option<&str>,
) -> Result<Option<Instance>> {
    if let Some(pane_id) = pane_id {
        let projected =
            super::target::load_pane(profile, id, pane_id, slot, launch_id, "opencode")?;
        let row = crate::db::Store::open_with_schema(profile)?
            .read_slots_for_instance(id)?
            .into_iter()
            .find(|row| row.slot == slot)
            .context("Managed OpenCode slot is missing")?;
        ensure!(
            row.xats_runtime_generation == generation,
            "Managed OpenCode slot generation is stale"
        );
        return Ok(Some(projected));
    }
    ensure!(
        launch_id.is_none(),
        "Managed OpenCode launch requires its logical pane ID"
    );
    let instances = Storage::new(profile)?.load()?;
    let Some(instance) = instances.into_iter().find(|item| item.id == id) else {
        ensure!(launch_id.is_none(), "Managed OpenCode session is missing");
        return Ok(None);
    };
    let managed_target = (slot == 0 && crate::session::managed_pane::has_managed_state(&instance))
        || instance
            .managed_panes
            .iter()
            .any(|pane| pane.slot == Some(slot));
    ensure!(
        !managed_target,
        "Managed OpenCode launch requires its logical pane ID"
    );
    Ok(None)
}

pub(crate) async fn bind(
    instance: &mut Instance,
    client: &reqwest::Client,
    base_url: &str,
    session_id: &str,
    directory: &std::path::Path,
) -> Result<()> {
    crate::managed_xats::bind(
        instance,
        json!({
            "agent_type": "opencode", "base_url": base_url, "session_id": session_id
        }),
    )?;
    let token = crate::managed_xats::mcp_token(instance)?;
    let control = crate::xats_control::discover_control_plane()?;
    let endpoint = control.base_url.join("/mcp")?.to_string();
    let result = configure(
        client,
        base_url,
        directory,
        &endpoint,
        token,
        control.token.as_ref().map(|value| value.expose()),
    )
    .await;
    crate::xats_control::redact_secrets(result, token, &control)
}

#[derive(Deserialize)]
struct Configuration {
    #[serde(default)]
    mcp: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct McpState {
    status: String,
    #[serde(default)]
    error: Option<String>,
}

async fn configure(
    client: &reqwest::Client,
    base_url: &str,
    directory: &std::path::Path,
    endpoint: &str,
    token: &str,
    bearer: Option<&str>,
) -> Result<()> {
    let config: Configuration = serde_json::from_value(
        read_response(client.get(endpoint_url(base_url, "config", directory)?)).await?,
    )
    .context("Invalid OpenCode configuration response")?;
    let (name, mut headers) = target(&config, endpoint)?;
    headers.insert("X-Xats-Managed-Token".to_string(), token.to_string());
    if let Some(bearer) = bearer {
        headers.insert("Authorization".to_string(), format!("Bearer {bearer}"));
    }
    let body = json!({"name": name, "config": {
        "type": "remote", "url": endpoint, "headers": headers,
        "enabled": true, "oauth": false
    }});
    let states: BTreeMap<String, McpState> = serde_json::from_value(
        read_response(
            client
                .post(endpoint_url(base_url, "mcp", directory)?)
                .json(&body),
        )
        .await?,
    )
    .context("Invalid OpenCode MCP response")?;
    let state = states
        .get(&name)
        .context("OpenCode omitted managed MCP status")?;
    ensure!(
        state.status == "connected",
        "OpenCode managed MCP failed: {} ({})",
        state.error.as_deref().unwrap_or(&state.status),
        state.status
    );
    Ok(())
}

fn endpoint_url(base_url: &str, path: &str, directory: &std::path::Path) -> Result<url::Url> {
    let mut url = url::Url::parse(&format!("{base_url}/{path}"))?;
    url.query_pairs_mut()
        .append_pair("directory", &directory.to_string_lossy());
    Ok(url)
}

fn target(config: &Configuration, endpoint: &str) -> Result<(String, BTreeMap<String, String>)> {
    let matches: Vec<_> = config
        .mcp
        .iter()
        .filter(|(_, entry)| entry.get("url").and_then(Value::as_str) == Some(endpoint))
        .collect();
    ensure!(
        matches.len() <= 1,
        "Multiple OpenCode MCP entries match the xats daemon"
    );
    if let Some((name, entry)) = matches.first() {
        let headers = entry
            .get("headers")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .context("Invalid existing OpenCode xats headers")?
            .unwrap_or_default();
        return Ok(((*name).clone(), headers));
    }
    ensure!(
        !config.mcp.contains_key(SERVER_NAME),
        "OpenCode managed MCP server name is already used by another endpoint"
    );
    Ok((SERVER_NAME.to_string(), BTreeMap::new()))
}

async fn read_response(request: reqwest::RequestBuilder) -> Result<Value> {
    let mut response = request
        .send()
        .await
        .context("Sending OpenCode managed MCP request")?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Reading OpenCode MCP response")?
    {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE_BYTES,
            "OpenCode MCP response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let value =
        serde_json::from_slice(&bytes).context("OpenCode managed MCP response is not JSON")?;
    ensure!(
        status.is_success(),
        "OpenCode managed MCP request failed (HTTP {status}): {}",
        error_detail(&value)
    );
    Ok(value)
}

fn error_detail(value: &Value) -> String {
    for key in ["message", "error"] {
        if let Some(message) = value.get(key).and_then(Value::as_str) {
            return message.to_string();
        }
    }
    let messages: Vec<_> = value
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|error| error.get("message").and_then(Value::as_str))
        .collect();
    if messages.is_empty() {
        "response has no error message".to_string()
    } else {
        messages.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_xats_headers_are_preserved_and_other_entries_untouched() {
        let config: Configuration = serde_json::from_value(json!({"mcp": {
            "xats": {"url": "http://127.0.0.1:9100/mcp", "headers": {"Custom": "keep"}},
            "other": {"url": "http://127.0.0.1:9200/mcp"}
        }}))
        .unwrap();
        let (name, headers) = target(&config, "http://127.0.0.1:9100/mcp").unwrap();
        assert_eq!(name, "xats");
        assert_eq!(headers.get("Custom").map(String::as_str), Some("keep"));
        assert_eq!(config.mcp.len(), 2);
    }

    #[test]
    fn ambiguous_xats_entries_fail_closed() {
        let config: Configuration = serde_json::from_value(json!({"mcp": {
            "one": {"url": "http://127.0.0.1:9100/mcp"},
            "two": {"url": "http://127.0.0.1:9100/mcp"}
        }}))
        .unwrap();
        assert!(target(&config, "http://127.0.0.1:9100/mcp").is_err());
    }
}
