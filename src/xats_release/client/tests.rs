use super::*;

fn response() -> Value {
    json!({
        "ok": true, "protocol_version": 1,
        "results": [
            {"index": 0, "state": "released", "agent": {
                "agent_id": "f5900ad2-2fd0-4f61-853c-6f0264c7b3fe",
                "name": "left", "team": "test", "agent_type": "codex"
            }},
            {"index": 1, "state": "already_unregistered", "agent": null}
        ]
    })
}

fn decode_value(value: &Value, count: usize) -> Result<()> {
    decode(
        StatusCode::OK,
        &serde_json::to_vec(value).unwrap(),
        &vec![false; count],
    )
}

#[test]
fn accepts_complete_ordered_release_and_idempotent_absence() {
    assert!(decode_value(&response(), 2).is_ok());
}

#[test]
fn dead_key_unclaimed_requires_the_matching_request_flag() {
    let mut value = response();
    value["results"][1]["state"] = json!("dead_key_unclaimed");
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(decode(StatusCode::OK, &bytes, &[false, true]).is_ok());
    assert!(decode(StatusCode::OK, &bytes, &[false, false]).is_err());
    assert!(decode(StatusCode::OK, &bytes, &[true, false]).is_err());
    assert!(decode(
        StatusCode::OK,
        &serde_json::to_vec(&response()).unwrap(),
        &[false, true]
    )
    .is_err());
    value["results"][1]["agent"] = value["results"][0]["agent"].clone();
    assert!(decode(
        StatusCode::OK,
        &serde_json::to_vec(&value).unwrap(),
        &[false, true]
    )
    .is_err());
}

#[test]
fn rejects_partial_reordered_and_contradictory_results() {
    assert!(decode_value(&response(), 1).is_err());
    for replacement in [
        json!({"index": 0, "state": "already_unregistered", "agent": null}),
        json!({"index": 1, "state": "released", "agent": null}),
        json!({"index": 1, "state": "already_unregistered"}),
    ] {
        let mut value = response();
        value["results"][1] = replacement;
        assert!(decode_value(&value, 2).is_err());
    }
}

#[test]
fn rejects_invalid_identity_and_unexpected_response_fields() {
    for (field, value) in [
        ("agent_id", json!("invalid")),
        ("name", json!("")),
        ("team", json!("bad\nteam")),
        ("token", json!("secret")),
    ] {
        let mut response = response();
        response["results"][0]["agent"][field] = value;
        assert!(decode_value(&response, 2).is_err());
    }
}

#[test]
fn preserves_specific_error_and_rejects_invalid_error_target() {
    let failure = json!({
        "ok": false, "protocol_version": 1, "error": "identity_mismatch",
        "detail": "identity moved to another pane", "target_index": 1,
        "retryable": false
    });
    let error = decode(
        StatusCode::CONFLICT,
        &serde_json::to_vec(&failure).unwrap(),
        &[false, false],
    )
    .unwrap_err();
    assert!(error.to_string().contains("identity_mismatch"));
    assert!(error.to_string().contains("identity moved to another pane"));
    assert!(decode_value(&failure, 1).is_err());
    assert!(decode(StatusCode::UNAUTHORIZED, b"{}", &[false, false]).is_err());
}

#[test]
fn sends_both_targets_in_one_authenticated_request() {
    use crate::xats_control::test_support::{control_for, spawn_fake_server, FakeResponse};
    use std::time::Duration;

    let server = spawn_fake_server(vec![FakeResponse {
        status: 200,
        body: response().to_string(),
        delay: Duration::ZERO,
    }]);
    let (_directory, control) = control_for(&server, Some("test-token"));
    let targets = ["%1", "%2"].map(|pane_id| Target {
        tmux: crate::xats_pane_lookup::PaneTarget {
            socket_path: "/tmp/private-tmux/socket".into(),
            server_pid: 100,
            server_started_at: 1_793_040_000,
            pane_id: pane_id.into(),
        },
        identity_key: Some(format!("key-{pane_id}")),
        opencode_session_id: None,
        allow_dead_if_identity_unclaimed: false,
    });
    xats_control::block_on_control("test-pane-release", move || {
        Box::pin(async move {
            let client = xats_control::build_control_client(Duration::from_secs(5))?;
            release(&control, &client, &targets).await
        })
    })
    .unwrap();
    let request = server.requests.recv().unwrap();
    assert!(request.contains("POST /api/runtime/panes/release "));
    assert!(request
        .to_lowercase()
        .contains("authorization: bearer test-token"));
    let value: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(value["targets"].as_array().unwrap().len(), 2);
    assert_eq!(value["targets"][0]["identity_key"], "key-%1");
    assert_eq!(value["targets"][1]["tmux"]["pane_id"], "%2");
    server.worker.join().unwrap();
}
