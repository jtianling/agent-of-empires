use super::*;
use crate::session::PaneConfig;

const THREAD: &str = "00000000-0000-4000-8000-000000000001";
const LAUNCH: &str = "00000000-0000-4000-8000-000000000002";

fn fixture() -> (tempfile::TempDir, Store, AgentSlot) {
    let root = tempfile::tempdir().unwrap();
    let (store, _) = Store::open_with_schema_at(&root.path().join("aoe.db")).unwrap();
    let pane = PaneConfig::new("codex", root.path().to_str().unwrap(), true, true);
    store
        .record_launched_slot_config_if_absent("instance", 0, &pane, "%1", "key", 1)
        .unwrap();
    let slot = store.read_slots_for_instance("instance").unwrap().remove(0);
    (root, store, slot)
}

#[test]
fn offline_recovery_preserves_explicit_provenance_and_requires_exact_resume_argv() {
    let (_root, store, slot) = fixture();
    assert!(store.recover_offline_codex_binding(&slot, THREAD).unwrap());
    let current = store.read_slots_for_instance("instance").unwrap().remove(0);
    let (state, launch): (String, String) = store
        .conn
        .query_row("SELECT state,launch_id FROM codex_binding", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!((state.as_str(), launch.as_str()), ("verified", ""));
    assert!(store.codex_resume_verified(&current).unwrap());
    assert!(store
        .codex_resume_launch_verified(&current, LAUNCH, Some(THREAD))
        .unwrap());
    assert!(!store
        .codex_resume_launch_verified(&current, LAUNCH, None)
        .unwrap());
    assert!(!store
        .codex_resume_launch_verified(&current, LAUNCH, Some(LAUNCH))
        .unwrap());
    let kind: String = store
        .conn
        .query_row("SELECT kind FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(kind, "codex_offline_recovery");
}

#[test]
fn offline_recovery_rejects_stale_slot_and_duplicate_thread() {
    let (_root, store, slot) = fixture();
    for stale in [
        AgentSlot {
            tmux_pane: "%2".into(),
            ..slot.clone()
        },
        AgentSlot {
            xats_identity_key: "other".into(),
            ..slot.clone()
        },
        AgentSlot {
            xats_runtime_generation: 1,
            ..slot.clone()
        },
    ] {
        assert!(!store.recover_offline_codex_binding(&stale, THREAD).unwrap());
    }
    assert!(store.recover_offline_codex_binding(&slot, THREAD).unwrap());
    store
        .conn
        .execute("UPDATE agent_slot SET instance_id = 'other'", [])
        .unwrap();
    let pane = slot.pane_config();
    store
        .record_launched_slot_config_if_absent("instance", 0, &pane, "%2", "new", 1)
        .unwrap();
    let other = store.read_slots_for_instance("instance").unwrap().remove(0);
    assert!(store.recover_offline_codex_binding(&other, THREAD).is_err());
}

#[test]
fn offline_recovery_rolls_back_when_audit_write_fails() {
    let (_root, store, slot) = fixture();
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER fail_recovery BEFORE INSERT ON events \
        BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;",
        )
        .unwrap();
    assert!(store.recover_offline_codex_binding(&slot, THREAD).is_err());
    assert_eq!(
        store.read_slots_for_instance("instance").unwrap().remove(0),
        slot
    );
    let count: i64 = store
        .conn
        .query_row("SELECT count(*) FROM codex_binding", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
