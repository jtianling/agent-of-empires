use super::*;

fn slot(slot: i64, pane: &str, key: &str) -> AgentSlot {
    AgentSlot {
        instance_id: "instance".into(),
        slot,
        agent: "codex".into(),
        native_session_id: String::new(),
        cwd: "/tmp".into(),
        launch_cwd: "/tmp".into(),
        tmux_pane: pane.into(),
        xats_identity_key: key.into(),
        xats_runtime_generation: 0,
        yolo_mode: false,
        cross_agent_team: !key.is_empty(),
        worktree_info: None,
        model: String::new(),
        model_fingerprint: String::new(),
        last_seen_at: 1,
    }
}

fn pane(id: &str, primary: bool) -> Pane {
    Pane {
        target: PaneTarget {
            socket_path: "/tmp/private-tmux/socket".into(),
            server_pid: 100,
            server_started_at: 1_793_040_000,
            pane_id: id.into(),
        },
        session: "session".into(),
        primary,
        marker: None,
        index: u32::from(!primary),
        left: 0,
        top: 0,
    }
}

#[test]
fn both_panes_use_their_own_slot_keys_and_ignore_stale_primary_mirror() {
    let mut instance = Instance::new("test", "/tmp");
    instance.xats_identity_key = Some("old-primary-key".into());
    let slots = [slot(0, "%1", "left-key"), slot(1, "%2", "right-key")];
    let plan = plan(&instance, &slots, &[pane("%1", true), pane("%2", false)]);
    assert_eq!(plan.targets[0].identity_key.as_deref(), Some("left-key"));
    assert_eq!(plan.targets[1].identity_key.as_deref(), Some("right-key"));
    assert!(plan.missing_keys.is_empty());
    assert!(plan.requires_daemon);
}

#[test]
fn right_only_and_hand_started_panes_remain_distinct() {
    let instance = Instance::new("test", "/tmp");
    let mut right = slot(1, "%2", "right-key");
    right.agent = "opencode".into();
    right.native_session_id = "ses_right".into();
    let plan = plan(
        &instance,
        &[right],
        &[pane("%1", true), pane("%2", false), pane("%3", false)],
    );
    assert!(plan.targets[0].identity_key.is_none());
    assert_eq!(
        plan.targets[1].opencode_session_id.as_deref(),
        Some("ses_right")
    );
    assert!(plan.targets[2].identity_key.is_none());
}

#[test]
fn excludes_fenced_managed_identities_and_keeps_missing_ordinary_keys() {
    let mut instance = Instance::new("test", "/tmp");
    instance.managed_xats =
        Some(crate::managed_xats::ManagedXats::new("worker".into(), "test".into()).unwrap());
    let slots = [slot(0, "%1", "managed-key"), slot(1, "%2", "missing-key")];
    let plan = plan(&instance, &slots, &[pane("%1", true)]);
    assert!(plan.targets.is_empty());
    assert_eq!(plan.missing_keys, ["missing-key"]);
}

#[test]
fn never_splits_an_oversized_atomic_release() {
    let instance = Instance::new("test", "/tmp");
    let panes = (0..=MAX_TARGETS)
        .map(|index| pane(&format!("%{index}"), index == 0))
        .collect::<Vec<_>>();
    let plan = plan(&instance, &[], &panes);
    assert!(crate::xats_release::validate_plan(&plan).is_err());
}
