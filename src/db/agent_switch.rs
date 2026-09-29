use anyhow::{bail, Result};
use rusqlite::params;

use super::Store;

impl Store {
    /// Hand every switchable slot of an instance to `target`, discarding the
    /// conversations those slots held.
    ///
    /// The identity key and runtime generation stay, so the new agent inherits
    /// the old one's xats identity. Pane captures and Codex bindings go: both
    /// describe the discarded conversation, and a leftover capture would let
    /// reconcile write the old agent back into the slot.
    pub fn switch_instance_agent(&self, instance_id: &str, target: &str) -> Result<()> {
        if !crate::agents::is_switchable_agent(target) {
            bail!("'{}' is not a switchable agent", target);
        }
        let transaction = self.conn.unchecked_transaction()?;
        let slots: Vec<(i64, String, String)> = {
            let mut statement = transaction
                .prepare("SELECT slot, agent, tmux_pane FROM agent_slot WHERE instance_id = ?1")?;
            let rows = statement.query_map([instance_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let switchable = slots
            .iter()
            .filter(|(_, agent, _)| crate::agents::is_switchable_agent(agent));
        for (slot, _, pane) in switchable {
            transaction.execute(
                "UPDATE agent_slot SET agent = ?1, native_session_id = '', model = '', \
                 model_fingerprint = '' WHERE instance_id = ?2 AND slot = ?3",
                params![target, instance_id, slot],
            )?;
            transaction.execute(
                "DELETE FROM codex_binding WHERE instance_id = ?1 AND slot = ?2",
                params![instance_id, slot],
            )?;
            if !pane.is_empty() {
                transaction.execute("DELETE FROM pane_live WHERE tmux_pane = ?1", [pane])?;
            }
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
        Store { conn }
    }

    #[test]
    fn switch_moves_agent_slots_and_keeps_identity() {
        let store = store();
        store
            .upsert_agent_slot("inst", 0, "claude", "sess-0", "/w", "%1", "key-0", 1)
            .unwrap();
        store
            .upsert_agent_slot("inst", 1, "codex", THREAD, "/w", "%2", "key-1", 1)
            .unwrap();
        store
            .upsert_agent_slot("inst", 2, "shell", "", "/w", "%3", "", 1)
            .unwrap();
        store
            .record_slot_model_probe("inst", 0, "fp", "opus")
            .unwrap();
        store
            .upsert_pane_live("%1", "claude", "sess-0", "/w", 1)
            .unwrap();
        store
            .upsert_pane_live("%3", "shell", "shell-sess", "/w", 1)
            .unwrap();
        let slots = store.read_slots_for_instance("inst").unwrap();
        store
            .capture_codex_binding(&slots[1], THREAD, THREAD)
            .unwrap();

        store.switch_instance_agent("inst", "codex").unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].agent, "codex");
        assert_eq!(slots[0].xats_identity_key, "key-0");
        assert!(slots[0].native_session_id.is_empty());
        assert!(slots[0].model.is_empty());
        assert!(slots[0].model_fingerprint.is_empty());
        assert_eq!(slots[1].agent, "codex");
        assert_eq!(slots[1].xats_identity_key, "key-1");
        assert!(slots[1].native_session_id.is_empty());
        assert_eq!(slots[2].agent, "shell");
        assert!(store.read_pane_live("%1").unwrap().is_none());
        assert!(store.read_pane_live("%2").unwrap().is_none());
        assert!(store.read_pane_live("%3").unwrap().is_some());
        let bindings: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM codex_binding", [], |row| row.get(0))
            .unwrap();
        assert_eq!(bindings, 0);
    }

    #[test]
    fn switch_rejects_unsupported_target() {
        let store = store();
        assert!(store.switch_instance_agent("inst", "opencode").is_err());
    }
}
