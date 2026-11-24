use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};

use super::Store;

impl Store {
    pub(crate) fn record_handoff_native(
        &self,
        id: &str,
        slot: i64,
        key: &str,
        tool: &str,
        native: &str,
    ) -> Result<()> {
        match tool {
            "codex" => {
                uuid::Uuid::parse_str(native)?;
            }
            "opencode" => crate::opencode_runtime::validate_session_id(native)?,
            _ => bail!("Unsupported handoff native capture"),
        }
        let changed = self.conn.execute(
            "UPDATE agent_slot SET native_session_id=?1 WHERE instance_id=?2 \
             AND slot=?3 AND agent=?4 AND xats_identity_key=?5",
            params![native, id, slot, tool, key],
        )?;
        anyhow::ensure!(changed == 1, "Handoff pane changed before native capture");
        Ok(())
    }
    pub(crate) fn align_opencode_runtime_generation(
        &self,
        id: &str,
        slot: i64,
        previous: i64,
        next: i64,
        key: &str,
    ) -> Result<()> {
        anyhow::ensure!(
            next > previous && next <= super::MAX_XATS_RUNTIME_GENERATION,
            "Invalid OpenCode runtime generation"
        );
        let changed = self.conn.execute(
            "UPDATE agent_slot SET xats_runtime_generation=?1 WHERE instance_id=?2 \
             AND slot=?3 AND agent='opencode' AND xats_runtime_generation=?4 \
             AND xats_identity_key=?5",
            params![next, id, slot, previous, key],
        )?;
        anyhow::ensure!(
            changed == 1,
            "OpenCode pane changed while aligning its generation"
        );
        Ok(())
    }
    /// Hand one switchable slot of an instance to `target`, discarding the
    /// conversation that slot held. Every other slot is left untouched.
    ///
    /// The identity key and runtime generation stay, so the new agent inherits
    /// the old one's xats identity. The pane capture and Codex binding go: both
    /// describe the discarded conversation, and a leftover capture would let
    /// reconcile write the old agent back into the slot.
    pub fn switch_slot_agent(&self, instance_id: &str, slot: i64, target: &str) -> Result<()> {
        if !crate::agents::is_switchable_agent(target) {
            bail!("'{}' is not a switchable agent", target);
        }
        let transaction = self.conn.unchecked_transaction()?;
        let row: Option<(String, String)> = transaction
            .query_row(
                "SELECT agent, tmux_pane FROM agent_slot WHERE instance_id = ?1 AND slot = ?2",
                params![instance_id, slot],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((agent, pane)) = row else {
            bail!("pane {} of the session is not tracked", slot + 1);
        };
        if !crate::agents::is_switchable_agent(&agent) {
            bail!(
                "pane {} runs {}, which cannot switch agents",
                slot + 1,
                agent
            );
        }
        transaction.execute(
            "UPDATE agent_slot SET agent = ?1, native_session_id = '', model = '', \
             model_fingerprint = '', runtime_settings = '' WHERE instance_id = ?2 AND slot = ?3",
            params![target, instance_id, slot],
        )?;
        transaction.execute(
            "DELETE FROM codex_binding WHERE instance_id = ?1 AND slot = ?2",
            params![instance_id, slot],
        )?;
        if !pane.is_empty() {
            transaction.execute("DELETE FROM pane_live WHERE tmux_pane = ?1", [pane])?;
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "00000000-0000-4000-8000-000000000001";

    fn store() -> Store {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        super::super::ensure_schema(&conn).unwrap();
        Store {
            conn,
            _inventory_snapshot: None,
        }
    }

    #[test]
    fn switch_moves_only_the_chosen_slot_and_keeps_identity() {
        let store = store();
        store
            .upsert_agent_slot("inst", 0, "claude", "sess-0", "/w", "%1", "key-0", 1)
            .unwrap();
        store
            .upsert_agent_slot("inst", 1, "codex", THREAD, "/w", "%2", "key-1", 1)
            .unwrap();
        store
            .record_slot_model_probe("inst", 0, "fp", "opus")
            .unwrap();
        store
            .upsert_pane_live("%1", "claude", "sess-0", "/w", 1)
            .unwrap();
        store
            .upsert_pane_live("%2", "codex", THREAD, "/w", 1)
            .unwrap();
        let slots = store.read_slots_for_instance("inst").unwrap();
        store
            .capture_codex_binding(&slots[1], THREAD, THREAD)
            .unwrap();

        store.switch_slot_agent("inst", 0, "codex").unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].agent, "codex");
        assert_eq!(slots[0].xats_identity_key, "key-0");
        assert!(slots[0].native_session_id.is_empty());
        assert!(slots[0].model.is_empty());
        assert!(slots[0].model_fingerprint.is_empty());
        assert_eq!(slots[1].agent, "codex");
        assert_eq!(slots[1].native_session_id, THREAD);
        assert!(store.read_pane_live("%1").unwrap().is_none());
        assert!(store.read_pane_live("%2").unwrap().is_some());
        let bindings: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM codex_binding", [], |row| row.get(0))
            .unwrap();
        assert_eq!(bindings, 1);

        store.switch_slot_agent("inst", 1, "claude").unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[1].agent, "claude");
        assert_eq!(slots[1].xats_identity_key, "key-1");
        assert!(slots[1].native_session_id.is_empty());
        assert!(store.read_pane_live("%2").unwrap().is_none());
        let bindings: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM codex_binding", [], |row| row.get(0))
            .unwrap();
        assert_eq!(bindings, 0);
    }

    #[test]
    fn switch_rejects_unsupported_target_missing_and_unswitchable_slots() {
        let store = store();
        store
            .upsert_agent_slot("inst", 0, "claude", "", "/w", "%1", "", 1)
            .unwrap();
        store
            .upsert_agent_slot("inst", 1, "shell", "", "/w", "%2", "", 1)
            .unwrap();
        assert!(store.switch_slot_agent("inst", 0, "shell").is_err());
        assert!(store.switch_slot_agent("inst", 1, "codex").is_err());
        assert!(store.switch_slot_agent("inst", 2, "codex").is_err());
        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].agent, "claude");
        assert_eq!(slots[1].agent, "shell");
    }

    #[test]
    fn opencode_switch_preserves_worktree_and_resumes_only_the_new_conversation() {
        let store = store();
        let directory = "/project/.worktrees/feature";
        let mut pane = crate::session::PaneConfig::new("claude", directory, true, true);
        pane.worktree = Some(crate::session::PaneWorktreeInfo {
            worktree_path: Some(directory.into()),
            ..Default::default()
        });
        store
            .upsert_agent_slot_config("inst", 1, &pane, "old-claude", "%2", "key", 1)
            .unwrap();
        store.switch_slot_agent("inst", 1, "opencode").unwrap();
        let prepared = store
            .prepare_opencode_runtime("inst", 1, crate::db::RuntimePreparationMode::Fresh)
            .unwrap();
        assert!(prepared.native_session_id.is_empty());
        assert_eq!(prepared.xats_identity_key, "key");
        assert!(store
            .record_opencode_runtime_session(
                "inst",
                1,
                prepared.generation,
                "%2",
                "ses_switched",
                directory,
                2
            )
            .unwrap());
        let resumed = store
            .prepare_opencode_runtime("inst", 1, crate::db::RuntimePreparationMode::Resume)
            .unwrap();
        assert_eq!(resumed.native_session_id, "ses_switched");
        let slot = store
            .read_slots_for_instance("inst")
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(slot.agent, "opencode");
        assert_eq!(slot.cwd, directory);
        assert_eq!(slot.worktree_info, pane.worktree);
        store.switch_slot_agent("inst", 1, "claude").unwrap();
        assert!(!store
            .record_opencode_runtime_session(
                "inst",
                1,
                resumed.generation,
                "%2",
                "ses_stale",
                directory,
                3
            )
            .unwrap());
        let slot = store
            .read_slots_for_instance("inst")
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(slot.agent, "claude");
        assert!(slot.native_session_id.is_empty());
        assert_eq!(slot.cwd, directory);
    }
}
