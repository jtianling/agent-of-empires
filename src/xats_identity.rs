use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::xats_control::{self, ControlPlane, HttpStatusError, PROTOCOL_VERSION};

const LOOKUP_PATH: &str = "/api/identity-key/lookup";

/// Who the xats daemon says holds an identity key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyHolder {
    Held {
        team: String,
        name: String,
        agent_type: Option<String>,
        active: bool,
    },
    NotFound,
    /// The daemon predates the lookup endpoint.
    Unsupported,
    Unavailable(String),
}

#[derive(Serialize)]
struct LookupRequest<'a> {
    protocol_version: u32,
    identity_key: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Found {
    ok: bool,
    protocol_version: u32,
    identity_key: String,
    holder: Holder,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Holder {
    team: String,
    name: String,
    agent_type: Option<String>,
    active: bool,
}

/// Look up the holder of each key, in order. Never fails as a whole: a
/// daemon that cannot be asked answers `Unavailable` for every key.
pub fn lookup_holders(keys: Vec<String>) -> Vec<KeyHolder> {
    if keys.is_empty() {
        return Vec::new();
    }
    let count = keys.len();
    let result = xats_control::discover_control_plane().and_then(|control| {
        xats_control::block_on_control("identity-key-lookup", move || {
            Box::pin(async move {
                let client = xats_control::build_control_client(xats_control::CONTROL_TIMEOUT)?;
                let mut holders = Vec::with_capacity(keys.len());
                for key in &keys {
                    holders.push(lookup_with(&control, &client, key).await);
                }
                Ok(holders)
            })
        })
    });
    result.unwrap_or_else(|e| vec![KeyHolder::Unavailable(format!("{e:#}")); count])
}

pub(crate) async fn lookup_with(
    control: &ControlPlane,
    client: &reqwest::Client,
    key: &str,
) -> KeyHolder {
    if let Err(e) = xats_control::validate_identity_key(key) {
        return KeyHolder::Unavailable(e.to_string());
    }
    let request = LookupRequest {
        protocol_version: PROTOCOL_VERSION,
        identity_key: key,
    };
    match xats_control::invoke(control, client, LOOKUP_PATH, &request).await {
        Ok(value) => {
            parse_lookup(key, value).unwrap_or_else(|e| KeyHolder::Unavailable(format!("{e:#}")))
        }
        Err(failure) => {
            let error = failure.into_error();
            if error
                .downcast_ref::<HttpStatusError>()
                .is_some_and(|status| status.0 == StatusCode::NOT_FOUND)
            {
                KeyHolder::Unsupported
            } else {
                KeyHolder::Unavailable(format!("{error:#}"))
            }
        }
    }
}

fn parse_lookup(key: &str, value: serde_json::Value) -> Result<KeyHolder> {
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        let code = xats_control::domain_error_code(&value)?;
        if code == "not_found" {
            return Ok(KeyHolder::NotFound);
        }
        anyhow::bail!("identity key lookup refused: {code}");
    }
    let found: Found = serde_json::from_value(value).context("invalid identity key lookup")?;
    if !found.ok || found.protocol_version != PROTOCOL_VERSION || found.identity_key != key {
        anyhow::bail!("identity key lookup response does not match the request");
    }
    Ok(KeyHolder::Held {
        team: found.holder.team,
        name: found.holder.name,
        agent_type: found.holder.agent_type,
        active: found.holder.active,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::xats_control::test_support::{control_for, spawn_fake_server, FakeResponse};

    fn respond(status: u16, body: &str) -> Vec<FakeResponse> {
        vec![FakeResponse {
            status,
            body: body.to_string(),
            delay: Duration::ZERO,
        }]
    }

    fn lookup(responses: Vec<FakeResponse>, key: &str) -> (KeyHolder, String) {
        let server = spawn_fake_server(responses);
        let (_dir, control) = control_for(&server, Some("token"));
        let key = key.to_string();
        let holder = xats_control::block_on_control("test-lookup", move || {
            Box::pin(async move {
                let client = xats_control::build_control_client(Duration::from_secs(5))?;
                Ok(lookup_with(&control, &client, &key).await)
            })
        })
        .unwrap();
        let request = server.requests.recv().unwrap();
        (holder, request)
    }

    #[test]
    fn held_key_reports_its_holder() {
        let (holder, request) = lookup(
            respond(
                200,
                r#"{"ok":true,"protocol_version":1,"identity_key":"key-1",
                    "holder":{"team":"mie","name":"mie-main","agent_type":"codex","active":true}}"#,
            ),
            "key-1",
        );
        assert_eq!(
            holder,
            KeyHolder::Held {
                team: "mie".to_string(),
                name: "mie-main".to_string(),
                agent_type: Some("codex".to_string()),
                active: true,
            }
        );
        assert!(request.contains("POST /api/identity-key/lookup"));
        assert!(request.contains(r#""identity_key":"key-1""#));
        assert!(request.to_ascii_lowercase().contains("bearer token"));
    }

    #[test]
    fn unheld_key_is_not_found() {
        let (holder, _) = lookup(respond(200, r#"{"ok":false,"error":"not_found"}"#), "key-1");
        assert_eq!(holder, KeyHolder::NotFound);
    }

    #[test]
    fn missing_endpoint_means_an_old_daemon_not_an_unheld_key() {
        let (holder, _) = lookup(respond(404, r#"{"error":"not found"}"#), "key-1");
        assert_eq!(holder, KeyHolder::Unsupported);
    }

    #[test]
    fn a_reply_for_another_key_is_rejected() {
        let (holder, _) = lookup(
            respond(
                200,
                r#"{"ok":true,"protocol_version":1,"identity_key":"other",
                    "holder":{"team":"t","name":"n","agent_type":null,"active":false}}"#,
            ),
            "key-1",
        );
        assert!(matches!(holder, KeyHolder::Unavailable(_)));
    }

    #[test]
    fn storage_failure_is_unavailable() {
        let (holder, _) = lookup(
            respond(503, r#"{"ok":false,"error":"storage_unavailable"}"#),
            "key-1",
        );
        assert!(matches!(holder, KeyHolder::Unavailable(_)));
    }
}
