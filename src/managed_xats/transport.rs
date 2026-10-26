use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::Fence;
use crate::xats_control::{self, ControlPlane};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Response {
    ok: bool,
    protocol_version: u32,
    pub fence: Fence,
    pub state: String,
    pub agent_id: Option<String>,
    pub runtime: Option<Value>,
    #[serde(default)]
    pub mcp_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    ok: bool,
    error: String,
    detail: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub(super) struct DomainFailure {
    pub code: String,
    detail: String,
}

pub(super) fn call(
    operation: &str,
    request: Value,
    key: &str,
    token: Option<&str>,
) -> Result<Response> {
    let control = xats_control::discover_control_plane()?;
    let operation = operation.to_string();
    let key = key.to_string();
    let token = token.map(str::to_string);
    xats_control::block_on_control("managed-xats", move || {
        Box::pin(async move {
            let result = invoke(&control, &operation, &request).await;
            let code = result.as_ref().err().and_then(|error| {
                error
                    .downcast_ref::<DomainFailure>()
                    .map(|error| error.code.clone())
            });
            let result = xats_control::redact_secrets(result, &key, &control);
            result.map_err(|error| {
                let detail = format!("{error:#}");
                let detail = token
                    .as_ref()
                    .filter(|token| !token.is_empty())
                    .map_or_else(|| detail.clone(), |token| detail.replace(token, "***"));
                tracing::error!(operation, "Managed xats failed: {detail}");
                match code {
                    Some(code) => anyhow::Error::new(DomainFailure { code, detail }),
                    None => anyhow::anyhow!(detail),
                }
            })
        })
    })
}

async fn invoke(control: &ControlPlane, operation: &str, request: &Value) -> Result<Response> {
    let client = xats_control::build_control_client(xats_control::CONTROL_TIMEOUT)?;
    let endpoint = control
        .base_url
        .join(&format!("/api/managed/{operation}"))?;
    let mut builder = client.post(endpoint).json(request);
    if let Some(token) = &control.token {
        builder = builder.bearer_auth(token.expose());
    }
    let mut response = builder
        .send()
        .await
        .context("sending managed xats request")?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("reading managed xats response")?
    {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= xats_control::MAX_RESPONSE_BYTES,
            "managed xats response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("managed xats returned invalid JSON (HTTP {status})"))?;
    if !status.is_success() || value.get("ok") == Some(&Value::Bool(false)) {
        let failure: Failure = serde_json::from_value(value)
            .with_context(|| format!("invalid managed xats error response (HTTP {status})"))?;
        ensure!(
            !failure.ok && !failure.error.is_empty(),
            "invalid xats failure schema"
        );
        return Err(DomainFailure {
            detail: format!(
                "xats {operation} failed (HTTP {status}, {}): {}",
                failure.error, failure.detail
            ),
            code: failure.error,
        }
        .into());
    }
    parse_response(operation, value)
}

fn parse_response(operation: &str, value: Value) -> Result<Response> {
    ensure!(
        value.get("agent_id").is_some() && value.get("runtime").is_some(),
        "managed xats response omitted required nullable fields"
    );
    let mut response: Response =
        serde_json::from_value(value).context("invalid managed xats response schema")?;
    ensure!(
        response.ok && response.protocol_version == 1,
        "invalid managed xats protocol"
    );
    response.fence.validate()?;
    let allowed = match operation {
        "prepare" => matches!(response.state.as_str(), "prepared" | "bound"),
        "bind" => response.state == "bound",
        "release" => response.state == "released",
        "status" => matches!(
            response.state.as_str(),
            "prepared" | "bound" | "ready" | "released"
        ),
        _ => false,
    };
    ensure!(allowed, "invalid managed xats state for {operation}");
    ensure!(
        (operation == "prepare") == response.mcp_token.is_some(),
        "invalid managed xats token response"
    );
    if matches!(response.state.as_str(), "bound" | "ready") {
        ensure!(
            response.agent_id.as_ref().is_some_and(|id| !id.is_empty())
                && response.runtime.is_some(),
            "bound xats response has no runtime"
        );
    }
    if response.state == "released" {
        ensure!(
            response.agent_id.is_none() && response.runtime.is_none(),
            "released xats response still holds a runtime"
        );
    }
    if response.state == "prepared" {
        ensure!(
            response.runtime.is_none(),
            "prepared xats response holds a runtime"
        );
    }
    if let Some(runtime) = &response.runtime {
        response.runtime = Some(canonical_runtime(runtime)?);
    }
    Ok(response)
}

#[derive(Deserialize)]
#[serde(tag = "agent_type", deny_unknown_fields)]
enum Runtime {
    #[serde(rename = "codex")]
    Codex { thread_id: String, ws_url: String },
    #[serde(rename = "opencode")]
    OpenCode {
        base_url: String,
        session_id: String,
    },
    #[serde(rename = "kimi-code")]
    Kimi {
        base_url: String,
        session_id: String,
    },
    #[serde(rename = "claude-code")]
    Claude {
        ui_pid: u32,
        channel_session_id: String,
    },
}

pub(super) fn validate_runtime(value: &Value) -> Result<()> {
    match serde_json::from_value(value.clone()).context("invalid managed runtime schema")? {
        Runtime::Codex { thread_id, ws_url } => {
            uuid::Uuid::try_parse(&thread_id).context("invalid Codex thread ID")?;
            validate_url(&ws_url, &["ws", "wss"])?;
        }
        Runtime::OpenCode {
            base_url,
            session_id,
        } => {
            validate_url(&base_url, &["http", "https"])?;
            ensure!(
                session_id.starts_with("ses") && session_id.len() > 3,
                "invalid OpenCode session ID"
            );
        }
        Runtime::Kimi {
            base_url,
            session_id,
        } => {
            validate_url(&base_url, &["http", "https"])?;
            ensure!(!session_id.trim().is_empty(), "empty Kimi session ID");
        }
        Runtime::Claude {
            ui_pid,
            channel_session_id,
        } => {
            ensure!(
                ui_pid > 1 && !channel_session_id.trim().is_empty(),
                "invalid Claude runtime coordinates"
            );
        }
    }
    Ok(())
}

fn validate_url(value: &str, schemes: &[&str]) -> Result<()> {
    let url = url::Url::parse(value).context("invalid managed runtime URL")?;
    ensure!(
        schemes.contains(&url.scheme())
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "invalid managed runtime endpoint"
    );
    Ok(())
}

pub(super) fn canonical_runtime(value: &Value) -> Result<Value> {
    validate_runtime(value)?;
    let mut runtime = value.clone();
    let field = match runtime["agent_type"].as_str() {
        Some("codex") => "ws_url",
        Some("opencode" | "kimi-code") => "base_url",
        _ => return Ok(runtime),
    };
    let raw = runtime[field]
        .as_str()
        .context("Missing runtime endpoint")?;
    let canonical = url::Url::parse(raw)?.to_string();
    runtime[field] = Value::String(if field == "base_url" {
        canonical.trim_end_matches('/').to_string()
    } else {
        canonical
    });
    Ok(runtime)
}

#[cfg(test)]
mod tests;
