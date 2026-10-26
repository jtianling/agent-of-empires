use super::*;
use crate::session::{PaneConfig, Storage};

fn add_bound(store: &crate::db::Store, parent: &mut Instance, pane_id: &str) {
    let mut pane = ManagedPane::new(
        uuid::Uuid::new_v4().to_string(),
        PaneConfig::new("shell", "/tmp", false, false),
        None,
    )
    .unwrap();
    let live = parent
        .managed_panes
        .iter()
        .filter_map(|pane| pane.tmux_pane.clone())
        .collect::<Vec<_>>();
    let (slot, reserved) = store
        .reserve_managed_pane_slot(&parent.id, &pane.config, &pane.identity_key, &live)
        .unwrap();
    store
        .bind_prepared_slot_pane(
            &parent.id,
            "shell",
            slot,
            reserved.generation,
            &pane.identity_key,
            "",
            pane_id,
            crate::db::now_unix(),
        )
        .unwrap();
    pane.slot = Some(slot);
    pane.slot_generation = reserved.generation;
    pane.tmux_pane = Some(pane_id.into());
    parent.managed_panes.push(pane);
}

#[test]
#[serial_test::serial]
fn failed_json_finalize_can_recover_committed_generation_and_remove_only_its_slot() {
    for rebind in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(temp.path());
        let storage = Storage::new("transition").unwrap();
        let store = crate::db::Store::open_with_schema("transition").unwrap();
        let mut parent = Instance::new("original", "/tmp");
        add_bound(&store, &mut parent, "%7");
        add_bound(&store, &mut parent, "%8");
        let neighbour = store
            .read_slots_for_instance(&parent.id)
            .unwrap()
            .into_iter()
            .find(|row| Some(row.slot) == parent.managed_panes[1].slot)
            .unwrap();
        let old_generation = parent.managed_panes[0].slot_generation;
        parent.managed_panes[0]
            .stage_slot_transition(true, rebind)
            .unwrap();
        storage.save(&[parent.clone()]).unwrap();

        let concurrent = Storage::new("transition").unwrap();
        let mut changed = concurrent.load().unwrap();
        changed[0].title = "concurrent title".into();
        concurrent.save(&changed).unwrap();
        assert!(recover_with_store(&store, &parent.id, &mut parent.managed_panes[0]).unwrap());
        assert!(storage.save(&[parent.clone()]).is_err());

        let retry = Storage::new("transition").unwrap();
        let mut durable = retry.load().unwrap();
        assert_eq!(durable[0].managed_panes[0].slot_generation, old_generation);
        assert!(durable[0].managed_panes[0]
            .pending_slot_transition
            .is_some());
        let database_before_guard = store.read_slots_for_instance(&parent.id).unwrap();
        let effective =
            effective_slot_record("transition", &durable[0], &durable[0].managed_panes[0]).unwrap();
        assert_eq!(effective.slot_generation, old_generation + 1);
        assert_eq!(
            store.read_slots_for_instance(&parent.id).unwrap(),
            database_before_guard
        );
        assert!(recover_pending_slot("transition", &mut durable[0], 0).unwrap());
        retry.save(&durable).unwrap();
        assert_eq!(durable[0].title, "concurrent title");
        assert!(!recover_pending_slot("transition", &mut durable[0], 0).unwrap());
        let recovered = &durable[0].managed_panes[0];
        assert_eq!(recovered.slot_generation, old_generation + 1);
        assert_eq!(
            recovered.tmux_pane.as_deref(),
            if rebind { None } else { Some("%7") }
        );
        store
            .delete_managed_pane_slot(
                &durable[0].id,
                recovered.slot.unwrap(),
                &recovered.identity_key,
                recovered.tmux_pane.as_deref().unwrap_or_default(),
                recovered.slot_generation,
            )
            .unwrap();
        durable[0].managed_panes.remove(0);
        retry.save(&durable).unwrap();
        assert_eq!(
            store.read_slots_for_instance(&durable[0].id).unwrap(),
            vec![neighbour]
        );
    }
}

#[test]
#[serial_test::serial]
fn pending_before_database_commit_replays_once_and_rejects_foreign_generation() {
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(temp.path());
    let store = crate::db::Store::open_with_schema("transition").unwrap();
    let mut parent = Instance::new("parent", "/tmp");
    add_bound(&store, &mut parent, "%7");
    parent.managed_panes[0]
        .stage_slot_transition(false, false)
        .unwrap();
    let pending = parent.managed_panes[0].clone();
    let database_before_guard = store.read_slots_for_instance(&parent.id).unwrap();
    let effective = effective_slot_record("transition", &parent, &pending).unwrap();
    assert_eq!(effective.slot_generation, pending.slot_generation);
    assert_eq!(
        store.read_slots_for_instance(&parent.id).unwrap(),
        database_before_guard
    );
    assert!(recover_pending_slot("transition", &mut parent, 0).unwrap());
    store
        .advance_managed_pane_slot(&parent.id, &parent.managed_panes[0], true, false)
        .unwrap();
    let mut stale = pending;
    assert!(recover_with_store(&store, &parent.id, &mut stale).is_err());
    assert!(stale.pending_slot_transition.is_some());
}

#[test]
#[serial_test::serial]
fn json_io_failure_does_not_lose_the_pending_database_transition() {
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(temp.path());
    let storage = Storage::new("transition-io").unwrap();
    let store = crate::db::Store::open_with_schema("transition-io").unwrap();
    let mut parent = Instance::new("parent", "/tmp");
    add_bound(&store, &mut parent, "%7");
    parent.managed_panes[0]
        .stage_slot_transition(true, false)
        .unwrap();
    storage.save(&[parent.clone()]).unwrap();
    recover_with_store(&store, &parent.id, &mut parent.managed_panes[0]).unwrap();
    let path = crate::session::get_profile_dir("transition-io")
        .unwrap()
        .join("sessions.json");
    let saved = path.with_extension("pending-fixture");
    std::fs::rename(&path, &saved).unwrap();
    std::fs::create_dir(&path).unwrap();
    let error = storage.save(&[parent.clone()]).unwrap_err();
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(saved, &path).unwrap();
    let retry = Storage::new("transition-io").unwrap();
    let mut durable = retry.load().unwrap();
    assert!(durable[0].managed_panes[0]
        .pending_slot_transition
        .is_some());
    assert!(recover_pending_slot("transition-io", &mut durable[0], 0).unwrap());
    retry.save(&durable).unwrap();
    assert_eq!(
        durable[0].managed_panes[0].slot_generation,
        parent.managed_panes[0].slot_generation
    );
}

#[test]
fn pending_transition_cannot_be_reinterpreted_under_another_launch_fence() {
    let identity = crate::managed_xats::ManagedXats::new("worker".into(), "test".into()).unwrap();
    let mut value = serde_json::to_value(&identity).unwrap();
    value["claim"] = serde_json::json!({
        "fence": {"claim_id": uuid::Uuid::new_v4().to_string(),
            "instance_id": identity.launch_instance_id, "generation": 1},
        "mcp_token": "fixture", "runtime": null
    });
    let mut pane = ManagedPane::new(
        uuid::Uuid::new_v4().to_string(),
        PaneConfig::new("claude", "/tmp", false, true),
        Some(serde_json::from_value(value).unwrap()),
    )
    .unwrap();
    pane.slot = Some(1);
    pane.slot_generation = 2;
    pane.stage_slot_transition(true, false).unwrap();
    assert!(pane.validate().is_ok());
    pane.xats
        .as_mut()
        .unwrap()
        .claim
        .as_mut()
        .unwrap()
        .fence
        .generation += 1;
    assert!(pane.validate().is_err());
}
