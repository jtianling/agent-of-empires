use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::types::Response;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    ok: bool,
    protocol_version: u32,
    error: String,
    detail: String,
    retryable: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("xats daemon does not expose the handoff {operation} endpoint (HTTP 404)")]
pub(crate) struct MissingEndpoint {
    pub operation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RouteNotFound {
    message: String,
    error: String,
    status_code: u16,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("xats handoff {code} (HTTP {status}, retryable={retryable}): {detail}")]
pub(crate) struct BusinessFailure {
    pub code: String,
    pub status: u16,
    pub detail: String,
    pub retryable: bool,
}

impl BusinessFailure {
    pub fn rejected_prepare(&self) -> bool {
        (400..500).contains(&self.status)
    }
}

pub(crate) fn call(operation: &str, request: Value, key: &str) -> Result<Response> {
    let control = crate::xats_control::discover_control_plane()?;
    let operation = operation.to_string();
    let key = key.to_string();
    crate::xats_control::block_on_control("runtime-handoff", move || {
        Box::pin(async move {
            let result = invoke(&control, &operation, &request).await;
            let business = result
                .as_ref()
                .err()
                .and_then(|error| error.downcast_ref::<BusinessFailure>())
                .cloned();
            let missing = result
                .as_ref()
                .err()
                .and_then(|error| error.downcast_ref::<MissingEndpoint>())
                .map(|missing| missing.operation.clone());
            let result = crate::xats_control::redact_secrets(result, &key, &control);
            if let Err(error) = &result {
                tracing::error!(operation, "Runtime handoff failed: {error:#}");
            }
            result.map_err(|error| match business {
                Some(mut failure) => {
                    failure.detail = format!("{error:#}");
                    anyhow::Error::new(failure)
                }
                None => match missing {
                    Some(operation) => MissingEndpoint { operation }.into(),
                    None => error,
                },
            })
        })
    })
}

async fn invoke(
    control: &crate::xats_control::ControlPlane,
    operation: &str,
    request: &Value,
) -> Result<Response> {
    let client = crate::xats_control::build_control_client(crate::xats_control::CONTROL_TIMEOUT)?;
    let path = format!("/api/runtime/handoff/{operation}");
    let mut builder = client.post(control.base_url.join(&path)?).json(request);
    if let Some(token) = &control.token {
        builder = builder.bearer_auth(token.expose());
    }
    let mut response = builder
        .send()
        .await
        .context("Calling xats runtime handoff")?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Reading xats handoff response")?
    {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= crate::xats_control::MAX_RESPONSE_BYTES,
            "xats handoff response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    decode(status.as_u16(), &bytes)
}

pub(super) fn decode(status: u16, bytes: &[u8]) -> Result<Response> {
    let value: Value = serde_json::from_slice(bytes)
        .with_context(|| format!("Invalid xats handoff response (HTTP {status})"))?;
    if matches!(status, 401 | 403) {
        anyhow::bail!("xats handoff authorization failed (HTTP {status}): {value}");
    }
    if status == 404 && value.get("protocol_version") != Some(&Value::from(1)) {
        if let Ok(route) = serde_json::from_value::<RouteNotFound>(value.clone()) {
            let operation = route
                .message
                .strip_prefix("Route POST:/api/runtime/handoff/")
                .and_then(|suffix| suffix.strip_suffix(" not found"));
            if route.status_code == 404 && route.error == "Not Found" {
                if let Some(operation @ ("prepare" | "commit" | "status" | "abort")) = operation {
                    return Err(MissingEndpoint {
                        operation: operation.to_string(),
                    }
                    .into());
                }
            }
        }
        anyhow::bail!("xats daemon does not expose the handoff endpoint (HTTP 404): {value}");
    }
    if status != 200 || value.get("ok") == Some(&Value::Bool(false)) {
        let failure: Failure = serde_json::from_value(value).with_context(|| {
            format!(
                "xats handoff endpoint is unavailable or returned \
                an incompatible failure (HTTP {status})"
            )
        })?;
        ensure!(
            !failure.ok
                && failure.protocol_version == 1
                && !failure.error.is_empty()
                && failure.error.len() <= 80
                && failure
                    .error
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
            "Invalid handoff failure"
        );
        return Err(BusinessFailure {
            code: failure.error,
            status,
            detail: failure.detail,
            retryable: failure.retryable,
        }
        .into());
    }
    let response: Response = serde_json::from_value(value).context("Invalid handoff response")?;
    response.validate()?;
    Ok(response)
}
