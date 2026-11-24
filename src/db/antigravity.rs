use anyhow::{ensure, Result};
use rusqlite::params;

use super::{AgentSlot, Store};
use crate::hooks::antigravity::valid_id;
use crate::session::PaneConfig;

impl Store {
    pub(crate) fn has_antigravity_slots(profile: &str, instance: &str) -> Result<bool> {
        let path = crate::session::get_app_dir_path()?
            .join("profiles")
            .join(profile)
            .join("aoe.db");
        if !path.try_exists()? {
            return Ok(false);
        }
        let conn = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_slot WHERE instance_id = ?1 AND agent = 'antigravity')",
            [instance], |row| row.get(0),
        )?)
    }

    pub(crate) fn open_antigravity_capture(profile: &str) -> Result<Option<Self>> {
        let path = super::db_path(profile)?;
        if !path.try_exists()? {
            return Ok(None);
        }
        let conn = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        conn.busy_timeout(std::time::Duration::ZERO)?;
        Ok(Some(Self {
            conn,
            _inventory_snapshot: None,
        }))
    }

    pub(crate) fn capture_antigravity_id(
        &self,
        instance: &str,
        pane: &str,
        id: &str,
    ) -> Result<bool> {
        ensure!(valid_id(id), "Invalid Antigravity conversation UUID");
        let tx = self.conn.unchecked_transaction()?;
        let count = tx.execute(
            "UPDATE agent_slot SET native_session_id = ?1, last_seen_at = ?2
             WHERE instance_id = ?3 AND tmux_pane = ?4 AND agent = 'antigravity'",
            params![id, super::now_unix(), instance, pane],
        )?;
        ensure!(count <= 1, "Ambiguous Antigravity pane binding");
        if count == 1 {
            self.delete_pane_live(pane)?;
        }
        tx.commit()?;
        Ok(count == 1)
    }

    pub(crate) fn relaunch_antigravity(
        &self,
        slot: &AgentSlot,
        config: &PaneConfig,
        new_pane: &str,
        resume: bool,
        launch: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        ensure!(
            self.read_slots_for_instance(&slot.instance_id)?
                .iter()
                .any(|s| s == slot),
            "Antigravity slot {} changed before relaunch",
            slot.slot
        );
        let id = if resume { &slot.native_session_id } else { "" };
        ensure!(
            !resume || valid_id(id),
            "Missing Antigravity conversation UUID"
        );
        self.upsert_agent_slot_config(
            &slot.instance_id,
            slot.slot,
            config,
            id,
            new_pane,
            &slot.xats_identity_key,
            super::now_unix(),
        )?;
        self.delete_pane_live(new_pane)?;
        launch()?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: &str = "2f52b5ab-0c1f-4121-9150-4595b571f562";
    const RIGHT: &str = "a194f116-b0d6-48b6-9071-6d9ad9b7221d";

    #[test]
    fn antigravity_same_cwd_concurrent_captures_survive_cold_rebind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aoe.db");
        let open = || Store::open_with_schema_at(&path).unwrap().0;
        let store = open();
        for (slot, pane, team) in [(0, "%41", false), (1, "%42", true)] {
            let config = PaneConfig::new("antigravity", "/same", false, team);
            store
                .record_launched_slot_config("s", slot, &config, pane, "", 1)
                .unwrap();
        }
        drop(store);
        std::thread::scope(|scope| {
            for (pane, id) in [("%42", RIGHT), ("%41", LEFT)] {
                scope.spawn(move || assert!(open().capture_antigravity_id("s", pane, id).unwrap()));
            }
        });
        let store = open();
        let slots = store.read_slots_for_instance("s").unwrap();
        assert_eq!(slots[0].native_session_id, LEFT);
        assert_eq!(slots[1].native_session_id, RIGHT);
        let right = &slots[1];
        assert!(store
            .relaunch_antigravity(right, &right.pane_config(), "%51", true, || anyhow::bail!(
                "synthetic launch failure"
            ))
            .is_err());
        assert_eq!(store.read_slots_for_instance("s").unwrap(), slots);
        store
            .relaunch_antigravity(right, &right.pane_config(), "%51", true, || Ok(()))
            .unwrap();
        assert!(!store.capture_antigravity_id("s", "%42", LEFT).unwrap());
        assert!(!store
            .capture_antigravity_id("another", "%51", LEFT)
            .unwrap());
        let fresh = &store.read_slots_for_instance("s").unwrap()[1];
        assert_eq!(fresh.native_session_id, RIGHT);
        store
            .relaunch_antigravity(fresh, &fresh.pane_config(), "%51", false, || Ok(()))
            .unwrap();
        for bad in [
            "",
            "latest",
            "--continue",
            "00000000-0000-0000-0000-000000000000",
            "2f52b5ab0c1f412191504595b571f562",
        ] {
            assert!(store.capture_antigravity_id("s", "%51", bad).is_err());
        }
        assert!(store.read_slots_for_instance("s").unwrap()[1]
            .native_session_id
            .is_empty());
    }
}
