use anyhow::{bail, Result};
use rusqlite::{params, TransactionBehavior};

use super::{AgentSlot, Store, MAX_XATS_RUNTIME_GENERATION};
use crate::session::RestartMode;

impl Store {
    pub(crate) fn codex_resume_verified(&self, slot: &AgentSlot) -> Result<bool> {
        if uuid::Uuid::try_parse(&slot.native_session_id).is_err() {
            return Ok(false);
        }
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM codex_binding WHERE instance_id = ?1 \
             AND slot = ?2 AND generation = ?3 AND thread_id = ?4 \
             AND identity_key = ?5)",
            params![
                slot.instance_id,
                slot.slot,
                slot.xats_runtime_generation,
                slot.native_session_id,
                slot.xats_identity_key
            ],
            |row| row.get(0),
        )?)
    }

    pub(crate) fn capture_codex_binding(
        &self,
        slot: &AgentSlot,
        launch_id: &str,
        thread_id: &str,
    ) -> Result<bool> {
        uuid::Uuid::try_parse(launch_id)?;
        uuid::Uuid::try_parse(thread_id)?;
        let transaction = self.conn.unchecked_transaction()?;
        let now = super::now_unix();
        if !write_capture(&transaction, slot, launch_id, thread_id, now)? {
            return Ok(false);
        }
        transaction.commit()?;
        Ok(true)
    }

    pub(crate) fn prepare_codex_restart(
        &self,
        slot: &AgentSlot,
        mode: RestartMode,
        target_pane: &str,
        retiring_launch: Option<&str>,
    ) -> Result<String> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if mode == RestartMode::Resume && !self.codex_resume_verified(slot)? {
            bail!(
                "Codex slot {} has no verified conversation to resume. \
                Wait for xats registration or use C to start a new conversation.",
                slot.slot
            );
        }
        let thread_id = match mode {
            RestartMode::Resume => slot.native_session_id.as_str(),
            RestartMode::Fresh => "",
        };
        write_restart(&transaction, slot, thread_id, target_pane, retiring_launch)?;
        transaction.commit()?;
        Ok(thread_id.to_string())
    }

    pub(crate) fn finish_codex_restart(&self, slot: &AgentSlot) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE codex_binding SET state = 'awaiting_launch' WHERE instance_id = ?1 AND slot = ?2 \
             AND generation = ?3 AND state = 'restarting'",
            params![
                slot.instance_id,
                slot.slot,
                slot.xats_runtime_generation + 1
            ],
        )?;
        if changed != 1 {
            bail!("Codex launch changed before restart completion");
        }
        Ok(())
    }

    pub(crate) fn abort_codex_restart(&self, slot: &AgentSlot, verified: bool) -> Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE agent_slot SET native_session_id = ?1 \
             WHERE instance_id = ?2 AND slot = ?3 AND xats_runtime_generation = ?4",
            params![
                slot.native_session_id,
                slot.instance_id,
                slot.slot,
                slot.xats_runtime_generation + 1
            ],
        )?;
        if changed != 1 {
            bail!("Codex launch changed before restart rollback");
        }
        if verified {
            transaction.execute(
                "UPDATE codex_binding SET thread_id = ?1, state = 'verified' \
                 WHERE instance_id = ?2 AND slot = ?3 AND generation = ?4",
                params![
                    slot.native_session_id,
                    slot.instance_id,
                    slot.slot,
                    slot.xats_runtime_generation + 1
                ],
            )?;
        } else {
            transaction.execute(
                "DELETE FROM codex_binding WHERE instance_id = ?1 AND slot = ?2",
                params![slot.instance_id, slot.slot],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn rebind_codex_slot(
        &self,
        slot: &AgentSlot,
        pane: &str,
        cwd: &str,
    ) -> Result<bool> {
        let transaction = self.conn.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE agent_slot SET tmux_pane = ?1, cwd = ?2, agent = 'codex', \
             native_session_id = '', xats_runtime_generation = xats_runtime_generation + 1 \
             WHERE instance_id = ?3 AND slot = ?4 AND tmux_pane = ?5 \
             AND xats_runtime_generation = ?6 AND xats_runtime_generation < ?7 \
             AND NOT EXISTS(SELECT 1 FROM codex_binding b WHERE b.instance_id = ?3 \
                 AND b.slot = ?4 AND b.state = 'restarting')",
            params![
                pane,
                cwd,
                slot.instance_id,
                slot.slot,
                slot.tmux_pane,
                slot.xats_runtime_generation,
                MAX_XATS_RUNTIME_GENERATION
            ],
        )?;
        if changed != 1 {
            return Ok(false);
        }
        transaction.execute(
            "DELETE FROM codex_binding WHERE instance_id = ?1 AND slot = ?2",
            params![slot.instance_id, slot.slot],
        )?;
        transaction.execute(
            "DELETE FROM pane_live WHERE tmux_pane IN (?1, ?2)",
            params![slot.tmux_pane, pane],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub(crate) fn record_failed_codex_recovery(
        &self,
        slot: &AgentSlot,
        target: &str,
    ) -> Result<()> {
        let current = self
            .read_slots_for_instance(&slot.instance_id)?
            .into_iter()
            .find(|current| current.slot == slot.slot)
            .ok_or_else(|| anyhow::anyhow!("Codex recovery slot disappeared"))?;
        if current.xats_runtime_generation != slot.xats_runtime_generation {
            if current.tmux_pane != target {
                bail!("Codex recovery slot was reassigned");
            }
            return Ok(());
        }
        let mode = if self.codex_resume_verified(slot)? {
            RestartMode::Resume
        } else {
            RestartMode::Fresh
        };
        self.prepare_codex_restart(slot, mode, target, None)?;
        self.finish_codex_restart(slot)
    }
}

fn write_capture(
    transaction: &rusqlite::Transaction<'_>,
    slot: &AgentSlot,
    launch_id: &str,
    thread_id: &str,
    now: i64,
) -> Result<bool> {
    let changed = transaction.execute(
            "UPDATE agent_slot SET native_session_id = ?1, last_seen_at = ?2 \
             WHERE instance_id = ?3 AND slot = ?4 AND agent = 'codex' \
             AND tmux_pane = ?5 AND xats_runtime_generation = ?6 \
             AND xats_identity_key = ?7 AND native_session_id = ?8 \
             AND NOT EXISTS(SELECT 1 FROM codex_binding b \
                 WHERE b.instance_id = ?3 AND b.slot = ?4 \
                 AND (b.state = 'restarting' OR (b.state = 'awaiting_launch' AND b.launch_id = ?9)))",
            params![
                thread_id,
                now,
                slot.instance_id,
                slot.slot,
                slot.tmux_pane,
                slot.xats_runtime_generation,
                slot.xats_identity_key,
                slot.native_session_id,
                launch_id
            ],
        )?;
    if changed != 1 {
        return Ok(false);
    }
    write_capture_proof(transaction, slot, launch_id, thread_id, now)?;
    Ok(true)
}

fn write_capture_proof(
    transaction: &rusqlite::Transaction<'_>,
    slot: &AgentSlot,
    launch_id: &str,
    thread_id: &str,
    now: i64,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO codex_binding \
             (instance_id, slot, generation, identity_key, launch_id, thread_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(instance_id, slot) DO UPDATE SET \
             generation = excluded.generation, identity_key = excluded.identity_key, \
             launch_id = excluded.launch_id, thread_id = excluded.thread_id, state = 'verified'",
        params![
            slot.instance_id,
            slot.slot,
            slot.xats_runtime_generation,
            slot.xats_identity_key,
            launch_id,
            thread_id
        ],
    )?;
    transaction.execute(
        "INSERT INTO pane_live VALUES (?1, 'codex', ?2, ?3, ?4) \
             ON CONFLICT(tmux_pane) DO UPDATE SET agent = 'codex', \
             native_session_id = excluded.native_session_id, cwd = excluded.cwd, \
             updated_at = excluded.updated_at",
        params![slot.tmux_pane, thread_id, slot.cwd, now],
    )?;
    if slot.native_session_id != thread_id {
        transaction.execute(
            "INSERT INTO events(instance_id, slot, kind, detail, created_at) \
                 VALUES (?1, ?2, 'codex_binding', ?3, ?4)",
            params![slot.instance_id, slot.slot, thread_id, now],
        )?;
    }
    Ok(())
}

fn write_restart(
    transaction: &rusqlite::Transaction<'_>,
    slot: &AgentSlot,
    thread_id: &str,
    target_pane: &str,
    retiring_launch: Option<&str>,
) -> Result<()> {
    let changed = transaction.execute(
        "UPDATE agent_slot SET xats_runtime_generation = \
             xats_runtime_generation + 1, native_session_id = ?1, tmux_pane = ?2 \
             WHERE instance_id = ?3 AND slot = ?4 AND agent = 'codex' \
             AND xats_runtime_generation = ?5 AND xats_runtime_generation < ?6 \
             AND native_session_id = ?7 AND tmux_pane = ?8 \
             AND xats_identity_key = ?9",
        params![
            thread_id,
            target_pane,
            slot.instance_id,
            slot.slot,
            slot.xats_runtime_generation,
            MAX_XATS_RUNTIME_GENERATION,
            slot.native_session_id,
            slot.tmux_pane,
            slot.xats_identity_key
        ],
    )?;
    if changed != 1 {
        bail!("Codex slot changed while preparing restart");
    }
    write_restart_fence(transaction, slot, thread_id, target_pane, retiring_launch)?;
    Ok(())
}

fn write_restart_fence(
    transaction: &rusqlite::Transaction<'_>,
    slot: &AgentSlot,
    thread_id: &str,
    target_pane: &str,
    retiring_launch: Option<&str>,
) -> Result<()> {
    transaction.execute(
        "DELETE FROM pane_live WHERE tmux_pane IN (?1, ?2)",
        params![slot.tmux_pane, target_pane],
    )?;
    transaction.execute(
        "INSERT INTO codex_binding \
             (instance_id, slot, generation, identity_key, launch_id, thread_id, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'restarting') \
             ON CONFLICT(instance_id, slot) DO UPDATE SET \
             generation = excluded.generation, thread_id = excluded.thread_id, \
             identity_key = excluded.identity_key, state = 'restarting', \
             launch_id = CASE WHEN ?5 != '' THEN ?5 ELSE codex_binding.launch_id END",
        params![
            slot.instance_id,
            slot.slot,
            slot.xats_runtime_generation + 1,
            slot.xats_identity_key,
            retiring_launch.unwrap_or_default(),
            thread_id
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: &str = "00000000-0000-4000-8000-000000000001";
    const RIGHT: &str = "00000000-0000-4000-8000-000000000002";

    fn store() -> Store {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        super::super::ensure_schema(&conn).unwrap();
        let store = Store { conn };
        for (index, pane) in [(0, "%18"), (1, "%19")] {
            store
                .record_launched_slot("instance", index, "codex", "/same/cwd", pane, pane, 1)
                .unwrap();
        }
        store
    }

    #[test]
    fn reversed_completion_keeps_each_panes_thread() {
        let store = store();
        let slots = store.read_slots_for_instance("instance").unwrap();
        store
            .capture_codex_binding(&slots[1], RIGHT, RIGHT)
            .unwrap();
        store.capture_codex_binding(&slots[0], LEFT, LEFT).unwrap();
        let slots = store.read_slots_for_instance("instance").unwrap();
        assert_eq!(
            store
                .prepare_codex_restart(&slots[0], RestartMode::Resume, "%30", Some(LEFT),)
                .unwrap(),
            LEFT
        );
        assert_eq!(
            store
                .prepare_codex_restart(&slots[1], RestartMode::Resume, "%31", Some(RIGHT),)
                .unwrap(),
            RIGHT
        );
    }

    #[test]
    fn fresh_rejects_late_capture_and_cannot_resume_old_thread() {
        let store = store();
        let old = store.read_slots_for_instance("instance").unwrap()[0].clone();
        store.capture_codex_binding(&old, LEFT, LEFT).unwrap();
        let before = store.read_slots_for_instance("instance").unwrap()[0].clone();
        store
            .prepare_codex_restart(&before, RestartMode::Fresh, "%18", Some(LEFT))
            .unwrap();
        assert!(!store.capture_codex_binding(&before, LEFT, LEFT).unwrap());
        let after = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert!(after.native_session_id.is_empty());
        assert!(store
            .prepare_codex_restart(&after, RestartMode::Resume, "%18", Some(LEFT),)
            .is_err());
        assert!(store.read_pane_live("%18").unwrap().is_none());
    }

    #[test]
    fn legacy_token_is_not_a_verified_binding() {
        let store = store();
        store
            .conn
            .execute("UPDATE agent_slot SET native_session_id = ?1", [LEFT])
            .unwrap();
        let slot = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert!(store
            .prepare_codex_restart(&slot, RestartMode::Resume, "%18", Some(LEFT),)
            .is_err());
        assert_eq!(store.read_slots_for_instance("instance").unwrap()[0], slot);
    }

    #[test]
    fn restart_fence_blocks_old_launch_even_with_the_new_generation() {
        let store = store();
        let slot = store.read_slots_for_instance("instance").unwrap()[0].clone();
        store.capture_codex_binding(&slot, LEFT, LEFT).unwrap();
        let slot = store.read_slots_for_instance("instance").unwrap()[0].clone();
        store
            .prepare_codex_restart(&slot, RestartMode::Fresh, "%18", Some(LEFT))
            .unwrap();
        let waiting = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert!(!store.capture_codex_binding(&waiting, RIGHT, RIGHT).unwrap());
        store.finish_codex_restart(&slot).unwrap();
        assert!(!store.capture_codex_binding(&waiting, LEFT, LEFT).unwrap());
        assert!(store.capture_codex_binding(&waiting, RIGHT, RIGHT).unwrap());
        let current = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert_eq!(current.native_session_id, RIGHT);
        assert!(store.codex_resume_verified(&current).unwrap());
    }

    #[test]
    fn failed_fresh_restart_restores_proof_without_rewinding_generation() {
        let store = store();
        let slot = store.read_slots_for_instance("instance").unwrap()[0].clone();
        store.capture_codex_binding(&slot, LEFT, LEFT).unwrap();
        let slot = store.read_slots_for_instance("instance").unwrap()[0].clone();
        store
            .prepare_codex_restart(&slot, RestartMode::Fresh, "%18", Some(LEFT))
            .unwrap();
        store.abort_codex_restart(&slot, true).unwrap();
        let current = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert_eq!(current.native_session_id, LEFT);
        assert_eq!(
            current.xats_runtime_generation,
            slot.xats_runtime_generation + 1
        );
        assert!(store.codex_resume_verified(&current).unwrap());
        assert!(!store.capture_codex_binding(&slot, LEFT, LEFT).unwrap());
    }

    #[test]
    fn stop_start_rebinding_discards_old_proof_and_inflight_capture() {
        let store = store();
        let slot = store.read_slots_for_instance("instance").unwrap()[0].clone();
        store.capture_codex_binding(&slot, LEFT, LEFT).unwrap();
        let slot = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert!(store.rebind_codex_slot(&slot, "%30", "/same/cwd").unwrap());
        let current = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert_eq!(current.tmux_pane, "%30");
        assert!(!store.codex_resume_verified(&current).unwrap());
        assert!(!store.capture_codex_binding(&slot, LEFT, LEFT).unwrap());
        assert!(store.capture_codex_binding(&current, RIGHT, RIGHT).unwrap());
        let current = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert!(store.codex_resume_verified(&current).unwrap());
    }

    #[test]
    fn old_observation_cannot_rebind_a_cold_restart_to_its_previous_pane() {
        let store = store();
        let old = store.read_slots_for_instance("instance").unwrap()[0].clone();
        store
            .prepare_codex_restart(&old, RestartMode::Fresh, "%30", Some(LEFT))
            .unwrap();
        assert!(!store.rebind_codex_slot(&old, "%18", "/same/cwd").unwrap());
        let during = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert!(!store
            .rebind_codex_slot(&during, "%18", "/same/cwd")
            .unwrap());
        store.finish_codex_restart(&old).unwrap();
        assert!(!store.rebind_codex_slot(&old, "%18", "/same/cwd").unwrap());
        let current = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert_eq!(current.tmux_pane, "%30");
        assert_eq!(
            current.xats_runtime_generation,
            old.xats_runtime_generation + 1
        );
    }

    #[test]
    fn rejected_legacy_cold_resume_keeps_the_new_placeholder_restartable() {
        let store = store();
        store
            .conn
            .execute("UPDATE agent_slot SET native_session_id = ?1", [LEFT])
            .unwrap();
        let old = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert!(store
            .prepare_codex_restart(&old, RestartMode::Resume, "%30", None,)
            .is_err());
        store.record_failed_codex_recovery(&old, "%30").unwrap();
        let current = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert_eq!(current.tmux_pane, "%30");
        assert!(current.native_session_id.is_empty());
        store
            .prepare_codex_restart(&current, RestartMode::Fresh, "%30", None)
            .unwrap();
        store.finish_codex_restart(&current).unwrap();
        let fresh = store.read_slots_for_instance("instance").unwrap()[0].clone();
        assert!(store.capture_codex_binding(&fresh, RIGHT, RIGHT).unwrap());
    }
}
