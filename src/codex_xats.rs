use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::xats_control::{self, PROTOCOL_VERSION};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct BindingRequest {
    pub protocol_version: u32,
    pub pane_id: String,
    pub launch_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    ok: bool,
    protocol_version: u32,
    pane_id: String,
    launch_id: String,
    thread_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unavailable {
    ok: bool,
    error: String,
}

pub(crate) fn lookup(request: BindingRequest) -> Result<Option<String>> {
    validate_request(&request)?;
    let control = xats_control::discover_control_plane()?;
    xats_control::block_on_control("codex-binding", move || {
        Box::pin(async move {
            let client = xats_control::build_control_client(xats_control::CONTROL_TIMEOUT)?;
            let value =
                xats_control::invoke(&control, &client, "/api/codex/binding/lookup", &request)
                    .await
                    .map_err(xats_control::ControlFailure::into_error)?;
            parse_binding(&request, value)
        })
    })
}

fn validate_request(request: &BindingRequest) -> Result<()> {
    let index = request.pane_id.strip_prefix('%').unwrap_or_default();
    if request.protocol_version != PROTOCOL_VERSION
        || index.is_empty()
        || !index.bytes().all(|byte| byte.is_ascii_digit())
        || uuid::Uuid::try_parse(&request.launch_id).is_err()
    {
        bail!("invalid Codex binding request");
    }
    Ok(())
}

fn parse_binding(request: &BindingRequest, value: serde_json::Value) -> Result<Option<String>> {
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        let unavailable: Unavailable =
            serde_json::from_value(value).context("invalid Codex binding refusal")?;
        if !unavailable.ok
            && matches!(
                unavailable.error.as_str(),
                "pending" | "not_found" | "stale" | "ambiguous"
            )
        {
            return Ok(None);
        }
        bail!("unexpected Codex binding refusal");
    }
    let binding: Binding =
        serde_json::from_value(value).context("invalid Codex binding response")?;
    if !binding.ok
        || binding.protocol_version != request.protocol_version
        || binding.pane_id != request.pane_id
        || binding.launch_id != request.launch_id
        || uuid::Uuid::try_parse(&binding.thread_id).is_err()
    {
        bail!("Codex binding response does not match this launch");
    }
    Ok(Some(binding.thread_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> BindingRequest {
        BindingRequest {
            protocol_version: 1,
            pane_id: "%18".to_string(),
            launch_id: "00000000-0000-4000-8000-000000000001".to_string(),
        }
    }

    #[test]
    fn rejects_sibling_and_previous_launch_responses() {
        for (pane, launch) in [
            ("%19", request().launch_id),
            ("%18", "00000000-0000-4000-8000-000000000002".to_string()),
        ] {
            assert!(parse_binding(
                &request(),
                json!({
                    "ok": true, "protocol_version": 1, "pane_id": pane,
                    "launch_id": launch,
                    "thread_id": "00000000-0000-4000-8000-000000000003"
                })
            )
            .is_err());
        }
    }

    #[test]
    fn pending_cannot_smuggle_a_previous_thread() {
        assert!(parse_binding(
            &request(),
            json!({
                "ok": false, "error": "pending", "thread_id": "old"
            })
        )
        .is_err());
        for error in ["pending", "not_found", "stale", "ambiguous"] {
            assert_eq!(
                parse_binding(
                    &request(),
                    json!({
                        "ok": false, "error": error
                    })
                )
                .unwrap(),
                None
            );
        }
    }

    #[test]
    fn accepts_only_the_requested_binding() {
        let thread = "00000000-0000-4000-8000-000000000003";
        assert_eq!(
            parse_binding(
                &request(),
                json!({
                    "ok": true, "protocol_version": 1, "pane_id": "%18",
                    "launch_id": request().launch_id, "thread_id": thread
                })
            )
            .unwrap()
            .as_deref(),
            Some(thread)
        );
    }
}
