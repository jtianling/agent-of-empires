use super::*;
use types::{Fence, Identity};

const ID: &str = "0123456789abcdef";
const KEY: &str = "00000000-0000-4000-8000-000000000001";
const HANDOFF: &str = "00000000-0000-4000-8000-000000000002";
const THREAD: &str = "00000000-0000-4000-8000-000000000003";

fn prepared() -> Response {
    let source = Identity {
        agent_id: "00000000-0000-4000-8000-000000000004".into(),
        name: "worker".into(),
        team: "project".into(),
        agent_type: Some("codex".into()),
        register_generation: 7,
        runtime_generation: None,
        runtime: Some(Runtime::Codex {
            thread_id: THREAD.into(),
            ws_url: "ws://127.0.0.1:8799/".into(),
        }),
    };
    Response {
        ok: true,
        protocol_version: 1,
        state: State::Prepared,
        fence: Fence {
            handoff_id: HANDOFF.into(),
            register_generation: 7,
        },
        current: Some(source.clone()),
        source,
        target_agent_type: "opencode".into(),
        recovery_prompt: None,
    }
}

fn pending() -> Pending {
    Pending {
        request_id: "00000000-0000-4000-8000-000000000005".into(),
        instance_id: ID.into(),
        slot: 0,
        identity_key: KEY.into(),
        name: "worker".into(),
        team: "project".into(),
        source_config: crate::session::PaneConfig::new("codex", "/tmp/worktree", false, true),
        source_native: THREAD.into(),
        source_pane: "%44".into(),
        source_launch: None,
        target_tool: "opencode".into(),
        codex_endpoint: None,
        response: Some(prepared()),
        target_runtime: None,
        attempt_id: None,
        controlled_identity: false,
        prepare_rejected: false,
    }
}

#[test]
#[serial_test::serial]
fn pending_state_is_private_and_preserves_source_across_process_restarts() {
    let root = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(root.path());
    let state = pending();
    pending::save("testing", &state).unwrap();
    let restored = pending::load("testing", ID, 0).unwrap().unwrap();
    assert_eq!(restored.identity_key, KEY);
    assert_eq!(restored.source_native, THREAD);
    assert_eq!(restored.source_config.working_dir, "/tmp/worktree");
    assert_eq!(restored.response.unwrap().fence.handoff_id, HANDOFF);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = crate::session::get_profile_dir("testing")
            .unwrap()
            .join("runtime-handoffs")
            .join(format!("{ID}.0.json"));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(pending::load("testing", ID, 0).is_err());
    }
}

#[test]
fn unrelated_identity_and_changed_fence_cannot_replace_pending_state() {
    let state = pending();
    let mut wrong = prepared();
    wrong.source.name = "someone-else".into();
    assert!(state.validate_response(&wrong).is_err());
    wrong = prepared();
    wrong.fence.register_generation += 1;
    assert!(state.validate_response(&wrong).is_err());
    wrong = prepared();
    wrong.target_agent_type = "claude-code".into();
    assert!(state.validate_response(&wrong).is_err());
}

#[test]
fn bootstrap_contains_only_local_helper_coordinates_and_never_the_identity_key() {
    let mut state = pending();
    state.target_tool = "codex".into();
    state.attempt_id = Some("00000000-0000-4000-8000-000000000006".into());
    state.response.as_mut().unwrap().target_agent_type = "codex".into();
    let prompt = complete::bootstrap_prompt("testing", &state).unwrap();
    assert!(!prompt.contains(KEY));
    assert!(!prompt.contains("completion_token"));
    assert!(prompt.contains(HANDOFF));
    assert!(prompt.contains("--attempt-id"));
    assert!(prompt.contains("$CODEX_THREAD_ID"));
}

#[test]
fn commit_runtime_matching_uses_exact_session_and_canonical_url() {
    let left = Runtime::OpenCode {
        session_id: "ses_new".into(),
        base_url: "http://127.0.0.1:9876".into(),
    };
    let right = Runtime::OpenCode {
        session_id: "ses_new".into(),
        base_url: "http://127.0.0.1:9876/".into(),
    };
    assert!(same_runtime(&left, &right));
    let other = Runtime::OpenCode {
        session_id: "ses_other".into(),
        base_url: "http://127.0.0.1:9876/".into(),
    };
    assert!(!same_runtime(&left, &other));
    let invalid = Runtime::Codex {
        thread_id: THREAD.into(),
        ws_url: "ws://user:secret@127.0.0.1:8799".into(),
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn malformed_or_extra_response_fields_are_rejected() {
    let mut value = serde_json::to_value(prepared()).unwrap();
    value["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Response>(value).is_err());
    let mut value = serde_json::to_value(prepared()).unwrap();
    value["state"] = serde_json::json!("maybe");
    assert!(serde_json::from_value::<Response>(value).is_err());
    let mut response = prepared();
    response.protocol_version = 2;
    assert!(response.validate().is_err());
}

#[test]
fn business_failures_preserve_the_daemon_reason_and_retry_contract() {
    let body = serde_json::to_vec(&serde_json::json!({"ok": false, "protocol_version": 1,
        "error": "runtime_probe_failed", "detail": "Exact session is not ready",
        "retryable": true}))
    .unwrap();
    let error = client::decode(502, &body).err().unwrap();
    let failure = error.downcast_ref::<client::BusinessFailure>().unwrap();
    assert_eq!(failure.code, "runtime_probe_failed");
    assert!(failure.retryable);
    assert!(failure.detail.contains("Exact session"));
    assert!(!failure.rejected_prepare());
    let body = serde_json::to_vec(&serde_json::json!({"ok": false, "protocol_version": 1,
        "error": "source_mismatch", "detail": "Source thread changed", "retryable": false}))
    .unwrap();
    let error = client::decode(409, &body).err().unwrap();
    assert!(error
        .downcast_ref::<client::BusinessFailure>()
        .unwrap()
        .rejected_prepare());
    let error = client::decode(404, b"{\"error\":\"Route not found\"}")
        .err()
        .unwrap();
    assert!(error.to_string().contains("does not expose"));
}

fn missing_prepare() -> anyhow::Error {
    client::decode(
        404,
        br#"{"message":"Route POST:/api/runtime/handoff/prepare not found",
        "error":"Not Found","statusCode":404}"#,
    )
    .err()
    .unwrap()
}

#[test]
#[serial_test::serial]
fn legacy_claude_codex_switch_discards_only_its_unaccepted_local_request() {
    let root = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(root.path());
    for (source, target) in [("claude", "codex"), ("codex", "claude")] {
        let mut state = pending();
        state.source_config.tool = source.into();
        state.target_tool = target.into();
        state.response = None;
        pending::save("testing", &state).unwrap();
        assert!(legacy::resume_legacy_switch("testing", &state, &missing_prepare()).unwrap());
        assert!(pending::load("testing", ID, 0).unwrap().is_none());
        assert_eq!(state.identity_key, KEY);
        assert_eq!(state.source_config.working_dir, "/tmp/worktree");
    }
}

#[test]
fn old_daemon_detection_requires_the_exact_missing_route_response() {
    assert!(missing_prepare()
        .downcast_ref::<client::MissingEndpoint>()
        .is_some());
    for (status, body) in [
        (
            404,
            br#"{"ok":false,"protocol_version":1,"error":"not_found",
            "detail":"Identity not found","retryable":false}"#
                .as_slice(),
        ),
        (
            404,
            br#"{"message":"Not found","error":"Not Found","statusCode":404}"#,
        ),
        (401, br#"{"message":"Unauthorized","statusCode":401}"#),
        (403, br#"{"error":"forbidden"}"#),
        (
            502,
            br#"{"ok":false,"protocol_version":1,"error":"runtime_probe_failed",
            "detail":"Not ready","retryable":true}"#,
        ),
    ] {
        let error = client::decode(status, body).err().unwrap();
        assert!(error.downcast_ref::<client::MissingEndpoint>().is_none());
    }
}

#[test]
#[serial_test::serial]
fn accepted_handoffs_and_opencode_never_use_legacy_recovery() {
    let root = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(root.path());
    let mut state = pending();
    state.target_tool = "claude".into();
    state.response.as_mut().unwrap().target_agent_type = "claude-code".into();
    pending::save("testing", &state).unwrap();
    assert!(!legacy::resume_legacy_switch("testing", &state, &missing_prepare()).unwrap());
    assert!(pending::load("testing", ID, 0)
        .unwrap()
        .unwrap()
        .response
        .is_some());
    state.response = None;
    state.controlled_identity = true;
    assert!(!legacy::resume_legacy_switch("testing", &state, &missing_prepare()).unwrap());
    state.controlled_identity = false;
    state.attempt_id = Some(HANDOFF.into());
    assert!(!legacy::resume_legacy_switch("testing", &state, &missing_prepare()).unwrap());
    state.attempt_id = None;
    state.target_runtime = Some(Runtime::Claude {
        ui_pid: 42,
        channel_session_id: None,
    });
    assert!(!legacy::resume_legacy_switch("testing", &state, &missing_prepare()).unwrap());
    state.target_runtime = None;
    state.target_tool = "opencode".into();
    assert!(!legacy::resume_legacy_switch("testing", &state, &missing_prepare()).unwrap());
    state.target_tool = "claude".into();
    let other_endpoint = client::MissingEndpoint {
        operation: "commit".into(),
    }
    .into();
    assert!(!legacy::resume_legacy_switch("testing", &state, &other_endpoint).unwrap());
    assert!(!legacy::resume_legacy_switch("testing", &state, &anyhow::anyhow!("timeout")).unwrap());
}
