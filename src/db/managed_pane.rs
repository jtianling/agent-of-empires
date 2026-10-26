use anyhow::{ensure, Result};
use rusqlite::{Transaction, TransactionBehavior};

use super::{
    select_available_extra_slot, serialize_pane_worktree, write_new_exact_session_slot,
    PreparedSlotRuntime, Store,
};

impl Store {
    pub fn rollback_managed_pane_binding(
        &self,
        instance: &str,
        pane: &crate::session::ManagedPane,
    ) -> Result<()> {
        let slot = pane
            .slot
            .ok_or_else(|| anyhow::anyhow!("Missing rollback slot"))?;
        let target = pane
            .tmux_pane
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Missing rollback pane"))?;
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE agent_slot SET tmux_pane='' WHERE instance_id=?1 AND slot=?2 \
             AND xats_identity_key=?3 AND xats_runtime_generation=?4 AND tmux_pane=?5",
            rusqlite::params![
                instance,
                slot,
                pane.identity_key,
                pane.slot_generation,
                target
            ],
        )?;
        ensure!(changed == 1, "Managed pane binding changed before rollback");
        transaction.execute(
            "DELETE FROM codex_binding WHERE instance_id=?1 AND slot=?2",
            rusqlite::params![instance, slot],
        )?;
        transaction.execute("DELETE FROM pane_live WHERE tmux_pane=?1", [target])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn advance_managed_pane_slot(
        &self,
        instance: &str,
        pane: &crate::session::ManagedPane,
        resume: bool,
        rebind: bool,
    ) -> Result<PreparedSlotRuntime> {
        let slot = pane
            .slot
            .ok_or_else(|| anyhow::anyhow!("Missing managed slot"))?;
        ensure!(
            pane.slot_generation < super::MAX_XATS_RUNTIME_GENERATION,
            "Managed pane generation exhausted"
        );
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let native: String = transaction.query_row(
            "SELECT native_session_id FROM agent_slot WHERE instance_id=?1 AND slot=?2 \
             AND xats_identity_key=?3 AND xats_runtime_generation=?4 AND tmux_pane=?5",
            rusqlite::params![
                instance,
                slot,
                pane.identity_key,
                pane.slot_generation,
                pane.tmux_pane.as_deref().unwrap_or("")
            ],
            |row| row.get(0),
        )?;
        let native = if resume { native } else { String::new() };
        let generation = pane.slot_generation + 1;
        transaction.execute(
            "UPDATE agent_slot SET xats_runtime_generation=?3, native_session_id=?4, \
             tmux_pane=?5, last_seen_at=?6 WHERE instance_id=?1 AND slot=?2",
            rusqlite::params![
                instance,
                slot,
                generation,
                native,
                if rebind {
                    ""
                } else {
                    pane.tmux_pane.as_deref().unwrap_or("")
                },
                super::now_unix()
            ],
        )?;
        transaction.execute(
            "DELETE FROM codex_binding WHERE instance_id=?1 AND slot=?2",
            rusqlite::params![instance, slot],
        )?;
        if !resume {
            if let Some(target) = &pane.tmux_pane {
                transaction.execute("DELETE FROM pane_live WHERE tmux_pane=?1", [target])?;
            }
        }
        transaction.commit()?;
        Ok(PreparedSlotRuntime {
            generation,
            native_session_id: native,
            xats_identity_key: pane.identity_key.clone(),
        })
    }

    pub fn reserve_managed_pane_slot(
        &self,
        instance_id: &str,
        pane: &crate::session::PaneConfig,
        identity_key: &str,
        live_panes: &[String],
    ) -> Result<(i64, PreparedSlotRuntime)> {
        pane.validate()?;
        let worktree = serialize_pane_worktree(pane.worktree.as_ref())?;
        let now = super::now_unix();
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let available =
            select_available_extra_slot(&transaction, instance_id, live_panes, now, now)?;
        let (slot, generation) = write_new_exact_session_slot(
            &transaction,
            available,
            instance_id,
            pane,
            identity_key,
            &worktree,
            now,
        )?;
        transaction.commit()?;
        Ok((
            slot,
            PreparedSlotRuntime {
                generation,
                native_session_id: String::new(),
                xats_identity_key: identity_key.to_string(),
            },
        ))
    }

    pub fn delete_managed_pane_slot(
        &self,
        instance: &str,
        slot: i64,
        key: &str,
        pane: &str,
        generation: i64,
    ) -> Result<()> {
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let current: Option<(String, String, i64)> = {
            use rusqlite::OptionalExtension;
            transaction
                .query_row(
                    "SELECT xats_identity_key, tmux_pane, xats_runtime_generation \
                 FROM agent_slot WHERE instance_id=?1 AND slot=?2",
                    rusqlite::params![instance, slot],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?
        };
        if let Some(current) = current {
            ensure!(
                current == (key.to_string(), pane.to_string(), generation),
                "Managed pane slot was replaced; refusing to delete it"
            );
            transaction.execute(
                "DELETE FROM codex_binding WHERE instance_id=?1 AND slot=?2",
                rusqlite::params![instance, slot],
            )?;
            transaction.execute(
                "DELETE FROM agent_slot WHERE instance_id=?1 AND slot=?2",
                rusqlite::params![instance, slot],
            )?;
            if !pane.is_empty() {
                transaction.execute("DELETE FROM pane_live WHERE tmux_pane=?1", [pane])?;
            }
            transaction.execute(
                "DELETE FROM instance_layout WHERE instance_id=?1",
                [instance],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn promote_managed_pane_slot(
        &self,
        instance: &str,
        session: &str,
        old_key: &str,
        old_pane: &str,
        candidate: &crate::session::ManagedPane,
    ) -> Result<()> {
        use rusqlite::OptionalExtension;
        let slot = candidate
            .slot
            .ok_or_else(|| anyhow::anyhow!("Missing promotion slot"))?;
        let pane = candidate
            .tmux_pane
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Missing promotion pane"))?;
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let read = |slot: i64| -> Result<Option<(String, String, i64)>> {
            Ok(transaction
                .query_row(
                    "SELECT xats_identity_key, tmux_pane, xats_runtime_generation \
                 FROM agent_slot WHERE instance_id=?1 AND slot=?2",
                    rusqlite::params![instance, slot],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?)
        };
        let expected = (
            candidate.identity_key.clone(),
            pane.to_string(),
            candidate.slot_generation,
        );
        if read(0)?.as_ref() == Some(&expected) && read(slot)?.is_none() {
            crate::tmux::set_agent_pane_id(session, pane)?;
            return Ok(());
        }
        let previous = read(0)?;
        ensure!(
            previous
                .as_ref()
                .is_some_and(|row| row.0 == old_key && row.1 == old_pane),
            "Primary slot changed before promotion"
        );
        ensure!(
            read(slot)?.as_ref() == Some(&expected),
            "Promotion candidate slot changed"
        );
        crate::tmux::set_agent_pane_id(session, pane)?;
        transaction.execute(
            "DELETE FROM codex_binding WHERE instance_id=?1 AND slot=0",
            [instance],
        )?;
        transaction.execute(
            "DELETE FROM agent_slot WHERE instance_id=?1 AND slot=0",
            [instance],
        )?;
        transaction.execute(
            "UPDATE agent_slot SET slot=0 WHERE instance_id=?1 AND slot=?2",
            rusqlite::params![instance, slot],
        )?;
        transaction.execute(
            "UPDATE codex_binding SET slot=0 WHERE instance_id=?1 AND slot=?2",
            rusqlite::params![instance, slot],
        )?;
        transaction.execute("DELETE FROM pane_live WHERE tmux_pane=?1", [old_pane])?;
        transaction.execute(
            "DELETE FROM instance_layout WHERE instance_id=?1",
            [instance],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn delete_retired_pane_records(&self, instance: &str, pane: &str) -> Result<()> {
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM codex_binding WHERE instance_id=?1 AND slot IN \
            (SELECT slot FROM agent_slot WHERE instance_id=?1 AND tmux_pane=?2)",
            rusqlite::params![instance, pane],
        )?;
        transaction.execute(
            "DELETE FROM agent_slot WHERE instance_id=?1 AND tmux_pane=?2",
            rusqlite::params![instance, pane],
        )?;
        transaction.execute("DELETE FROM pane_live WHERE tmux_pane=?1", [pane])?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_and_deletions_are_fenced_per_slot() {
        let directory = tempfile::tempdir().unwrap();
        let (store, _) = Store::open_with_schema_at(&directory.path().join("store.db")).unwrap();
        let config = crate::session::PaneConfig::new("shell", "/tmp", false, false);
        let (first, a) = store
            .reserve_managed_pane_slot("parent", &config, "a", &[])
            .unwrap();
        let (second, b) = store
            .reserve_managed_pane_slot("parent", &config, "b", &[])
            .unwrap();
        assert_ne!(first, second);
        store
            .bind_prepared_slot_pane("parent", "shell", first, a.generation, "a", "", "%1", 1)
            .unwrap();
        assert!(store
            .delete_managed_pane_slot("parent", first, "wrong", "%1", a.generation)
            .is_err());
        assert!(store
            .delete_managed_pane_slot("parent", first, "a", "%1", a.generation + 1)
            .is_err());
        store
            .delete_managed_pane_slot("parent", first, "a", "%1", a.generation)
            .unwrap();
        let rows = store.read_slots_for_instance("parent").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slot, second);
        assert_eq!(rows[0].xats_runtime_generation, b.generation);
    }

    #[test]
    fn stale_binding_cannot_advance_or_rollback_a_reused_slot() {
        let directory = tempfile::tempdir().unwrap();
        let (store, _) = Store::open_with_schema_at(&directory.path().join("store.db")).unwrap();
        let config = crate::session::PaneConfig::new("shell", "/tmp", false, false);
        let mut pane =
            crate::session::ManagedPane::new(uuid::Uuid::new_v4().to_string(), config, None)
                .unwrap();
        let (slot, reserved) = store
            .reserve_managed_pane_slot("parent", &pane.config, &pane.identity_key, &[])
            .unwrap();
        pane.slot = Some(slot);
        pane.slot_generation = reserved.generation;
        pane.tmux_pane = Some("%1".into());
        store
            .bind_prepared_slot_pane(
                "parent",
                "shell",
                slot,
                pane.slot_generation,
                &pane.identity_key,
                "",
                "%1",
                1,
            )
            .unwrap();
        let advanced = store
            .advance_managed_pane_slot("parent", &pane, true, false)
            .unwrap();
        assert!(store
            .rollback_managed_pane_binding("parent", &pane)
            .is_err());
        assert!(store
            .advance_managed_pane_slot("parent", &pane, true, false)
            .is_err());
        pane.slot_generation = advanced.generation;
        store
            .rollback_managed_pane_binding("parent", &pane)
            .unwrap();
        assert!(store.read_slots_for_instance("parent").unwrap()[0]
            .tmux_pane
            .is_empty());
    }
}
