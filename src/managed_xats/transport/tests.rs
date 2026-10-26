use super::*;
use serde_json::json;

fn response(state: &str) -> Value {
    json!({
        "ok": true, "protocol_version": 1,
        "fence": {
            "claim_id": "00000000-0000-4000-8000-000000000001",
            "instance_id": "00000000-0000-4000-8000-000000000002",
            "generation": 1
        },
        "state": state, "agent_id": null, "runtime": null
    })
}

#[test]
fn prepared_is_not_ready_and_requires_its_token() {
    assert!(parse_response("prepare", response("prepared")).is_err());
    let mut value = response("prepared");
    value["mcp_token"] = json!("test-token");
    assert_eq!(parse_response("prepare", value).unwrap().state, "prepared");
    assert!(parse_response("status", response("ready")).is_err());
}

#[test]
fn rejects_wrong_protocol_fence_and_unexpected_fields() {
    for (field, invalid) in [
        ("protocol_version", json!(2)),
        ("ok", json!(false)),
        ("unexpected", json!("field")),
    ] {
        let mut value = response("prepared");
        value[field] = invalid;
        assert!(parse_response("status", value).is_err());
    }
    let mut value = response("prepared");
    value["fence"]["generation"] = json!(0);
    assert!(parse_response("status", value).is_err());
    let mut value = response("prepared");
    value["fence"]["instance_id"] = json!("not-a-uuid");
    assert!(parse_response("status", value).is_err());
}

#[test]
fn bound_requires_schema_valid_exact_runtime() {
    let mut value = response("bound");
    value["agent_id"] = json!("agent");
    value["runtime"] = json!({
        "agent_type": "codex",
        "thread_id": "00000000-0000-4000-8000-000000000003",
        "ws_url": "ws://127.0.0.1:8799"
    });
    assert!(parse_response("bind", value.clone()).is_ok());
    value["runtime"]["latest"] = json!(true);
    assert!(parse_response("bind", value).is_err());
}

#[test]
fn rejects_secret_in_status_and_runtime_in_released_response() {
    let mut value = response("prepared");
    value["mcp_token"] = json!("test-token");
    assert!(parse_response("status", value).is_err());
    let mut value = response("released");
    value["agent_id"] = json!("still-owned");
    assert!(parse_response("release", value).is_err());
}

#[test]
fn runtime_schema_refuses_ambiguous_coordinates() {
    for value in [
        json!({"agent_type":"codex", "thread_id":"latest",
            "ws_url":"ws://127.0.0.1:8799"}),
        json!({"agent_type":"opencode", "session_id":"latest",
            "base_url":"http://127.0.0.1:1234"}),
        json!({"agent_type":"kimi-code", "session_id":" ",
            "base_url":"http://127.0.0.1:1234"}),
        json!({"agent_type":"claude-code", "ui_pid":0,
            "channel_session_id":"session"}),
    ] {
        assert!(validate_runtime(&value).is_err());
    }
}

#[test]
fn canonical_urls_preserve_exact_runtime_identity() {
    let mut value = json!({
        "agent_type": "codex",
        "thread_id": "00000000-0000-4000-8000-000000000003",
        "ws_url": "ws://LOCALHOST:80"
    });
    let canonical = canonical_runtime(&value).unwrap();
    assert_eq!(canonical["ws_url"], "ws://localhost/");
    value["ws_url"] = json!("ws://localhost/");
    assert_eq!(canonical_runtime(&value).unwrap(), canonical);
    value["thread_id"] = json!("00000000-0000-4000-8000-000000000004");
    assert_ne!(canonical_runtime(&value).unwrap(), canonical);
    let http = json!({"agent_type":"opencode", "session_id":"ses_test",
        "base_url":"http://LOCALHOST:80/"});
    assert_eq!(
        canonical_runtime(&http).unwrap()["base_url"],
        "http://localhost"
    );
}
