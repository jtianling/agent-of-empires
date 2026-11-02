use anyhow::{ensure, Result};

use super::{AgentSlot, Store};

impl Store {
    pub(crate) fn capture_managed_claude_session(
        &self,
        slot: &AgentSlot,
        expected: &str,
    ) -> Result<()> {
        uuid::Uuid::try_parse(expected)?;
        ensure!(slot.agent == "claude", "Managed slot is not Claude");
        if slot.native_session_id == expected {
            return Ok(());
        }
        let Some(capture) = self.read_pane_live(&slot.tmux_pane)? else {
            return Ok(());
        };
        if capture.agent == "claude"
            && capture.native_session_id == slot.native_session_id
            && uuid::Uuid::try_parse(&slot.native_session_id).is_ok()
        {
            return Ok(());
        }
        ensure!(
            capture.agent == "claude" && capture.native_session_id == expected,
            "Claude capture does not match the saved conversation"
        );
        let changed = self.conn.execute(
            "UPDATE agent_slot SET native_session_id=?1, last_seen_at=?2 \
             WHERE instance_id=?3 AND slot=?4 AND agent='claude' \
             AND tmux_pane=?5 AND xats_identity_key=?6 \
             AND xats_runtime_generation=?7 AND native_session_id=?8",
            rusqlite::params![
                expected,
                super::now_unix(),
                slot.instance_id,
                slot.slot,
                slot.tmux_pane,
                slot.xats_identity_key,
                slot.xats_runtime_generation,
                slot.native_session_id,
            ],
        )?;
        ensure!(
            changed == 1,
            "Managed Claude slot binding changed during capture"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Store, AgentSlot, String) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open_at(&directory.path().join("aoe.db")).unwrap();
        super::super::ensure_schema(&store.conn).unwrap();
        let config = crate::session::PaneConfig::new("claude", "/tmp", true, true);
        store
            .upsert_agent_slot_config("parent", 0, &config, "", "%4", "key", 1)
            .unwrap();
        let native = uuid::Uuid::new_v4().to_string();
        store
            .upsert_pane_live("%4", "claude", &native, "/tmp", 2)
            .unwrap();
        let slot = store.read_slots_for_instance("parent").unwrap().remove(0);
        (directory, store, slot, native)
    }

    #[test]
    fn capture_persists_conversation_without_changing_runtime_or_neighbor() {
        let (_directory, store, slot, native) = fixture();
        let config = crate::session::PaneConfig::new("claude", "/tmp", false, false);
        store
            .upsert_agent_slot_config("neighbor", 0, &config, "", "%5", "other", 1)
            .unwrap();
        let neighbor = store.read_slots_for_instance("neighbor").unwrap();
        store
            .capture_managed_claude_session(&slot, &native)
            .unwrap();
        let actual = store.read_slots_for_instance("parent").unwrap().remove(0);
        assert_eq!(actual.native_session_id, native);
        assert_eq!(actual.tmux_pane, slot.tmux_pane);
        assert_eq!(actual.xats_identity_key, slot.xats_identity_key);
        assert_eq!(actual.xats_runtime_generation, slot.xats_runtime_generation);
        assert_eq!(actual.yolo_mode, slot.yolo_mode);
        assert_eq!(actual.cross_agent_team, slot.cross_agent_team);
        assert_eq!(store.read_slots_for_instance("neighbor").unwrap(), neighbor);
        store
            .capture_managed_claude_session(&actual, &native)
            .unwrap();
    }

    #[test]
    fn confirmed_fresh_conversation_replaces_the_prior_slot_conversation() {
        let (_directory, store, slot, native) = fixture();
        let previous = uuid::Uuid::new_v4().to_string();
        store
            .conn
            .execute("UPDATE agent_slot SET native_session_id=?1", [&previous])
            .unwrap();
        let old = store.read_slots_for_instance("parent").unwrap().remove(0);
        store.capture_managed_claude_session(&old, &native).unwrap();
        let actual = store.read_slots_for_instance("parent").unwrap().remove(0);
        assert_eq!(actual.native_session_id, native);
        assert_eq!(actual.xats_identity_key, slot.xats_identity_key);
        assert_eq!(actual.xats_runtime_generation, slot.xats_runtime_generation);
    }

    #[test]
    fn prior_capture_allows_retry_after_a_fresh_launch_failure() {
        let (_directory, store, _slot, expected) = fixture();
        let previous = uuid::Uuid::new_v4().to_string();
        store
            .conn
            .execute("UPDATE agent_slot SET native_session_id=?1", [&previous])
            .unwrap();
        store
            .upsert_pane_live("%4", "claude", &previous, "/tmp", 3)
            .unwrap();
        let old = store.read_slots_for_instance("parent").unwrap().remove(0);
        store
            .capture_managed_claude_session(&old, &expected)
            .unwrap();
        assert_eq!(
            store.read_slots_for_instance("parent").unwrap()[0].native_session_id,
            previous
        );
        store
            .upsert_pane_live("%4", "codex", &previous, "/tmp", 4)
            .unwrap();
        assert!(store
            .capture_managed_claude_session(&old, &expected)
            .is_err());
        store
            .upsert_pane_live("%4", "claude", &expected, "/tmp", 5)
            .unwrap();
        store
            .capture_managed_claude_session(&old, &expected)
            .unwrap();
        assert_eq!(
            store.read_slots_for_instance("parent").unwrap()[0].native_session_id,
            expected
        );
    }

    #[test]
    fn stale_binding_cannot_capture_into_a_changed_slot() {
        for change in [
            "tmux_pane='%9'",
            "xats_identity_key='replacement'",
            "xats_runtime_generation=1",
            "native_session_id='replacement'",
        ] {
            let (_directory, store, slot, native) = fixture();
            store
                .conn
                .execute(&format!("UPDATE agent_slot SET {change}"), [])
                .unwrap();
            let before = store.read_slots_for_instance("parent").unwrap();
            assert!(store
                .capture_managed_claude_session(&slot, &native)
                .is_err());
            assert_eq!(store.read_slots_for_instance("parent").unwrap(), before);
        }
    }

    #[test]
    fn wrong_capture_or_storage_failure_does_not_become_a_fresh_session() {
        let (_directory, store, slot, native) = fixture();
        store
            .upsert_pane_live("%4", "claude", &uuid::Uuid::new_v4().to_string(), "/tmp", 3)
            .unwrap();
        assert!(store
            .capture_managed_claude_session(&slot, &native)
            .is_err());
        assert!(store.read_slots_for_instance("parent").unwrap()[0]
            .native_session_id
            .is_empty());
        store.conn.execute("DROP TABLE pane_live", []).unwrap();
        let error = store
            .capture_managed_claude_session(&slot, &native)
            .unwrap_err();
        assert!(error.to_string().contains("no such table: pane_live"));
    }
}
