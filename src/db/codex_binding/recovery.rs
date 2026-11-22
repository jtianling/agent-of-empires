use anyhow::{ensure, Result};
use rusqlite::{params, Transaction, TransactionBehavior};

use crate::db::{AgentSlot, Store};

impl Store {
    pub(crate) fn recover_offline_codex_binding(
        &self,
        slot: &AgentSlot,
        thread: &str,
    ) -> Result<bool> {
        uuid::Uuid::parse_str(thread)?;
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let duplicate: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_slot WHERE agent = 'codex' \
             AND native_session_id = ?1 AND (instance_id != ?2 OR slot != ?3))",
            params![thread, slot.instance_id, slot.slot],
            |row| row.get(0),
        )?;
        ensure!(
            !duplicate,
            "Another slot already records the selected conversation"
        );
        if !bind_slot(&transaction, slot, thread)? {
            return Ok(false);
        }
        write_proof(&transaction, slot, thread)?;
        transaction.commit()?;
        Ok(true)
    }
}

fn bind_slot(transaction: &Transaction<'_>, slot: &AgentSlot, thread: &str) -> Result<bool> {
    let changed = transaction.execute(
        "UPDATE agent_slot SET native_session_id = ?1 \
             WHERE instance_id = ?2 AND slot = ?3 AND agent = 'codex' \
             AND tmux_pane = ?4 AND xats_runtime_generation = ?5 \
             AND xats_identity_key = ?6 AND native_session_id = ?7 \
             AND NOT EXISTS(SELECT 1 FROM codex_binding b WHERE b.instance_id = ?2 \
                 AND b.slot = ?3 AND b.state = 'restarting')",
        params![
            thread,
            slot.instance_id,
            slot.slot,
            slot.tmux_pane,
            slot.xats_runtime_generation,
            slot.xats_identity_key,
            slot.native_session_id
        ],
    )?;
    Ok(changed == 1)
}

fn write_proof(transaction: &Transaction<'_>, slot: &AgentSlot, thread: &str) -> Result<()> {
    transaction.execute(
        "INSERT INTO codex_binding(instance_id, slot, generation, identity_key, \
             launch_id, thread_id, state) VALUES (?1, ?2, ?3, ?4, '', ?5, 'verified') \
             ON CONFLICT(instance_id, slot) DO UPDATE SET generation = excluded.generation, \
             identity_key = excluded.identity_key, launch_id = '', thread_id = excluded.thread_id, \
             state = 'verified'",
        params![
            slot.instance_id,
            slot.slot,
            slot.xats_runtime_generation,
            slot.xats_identity_key,
            thread
        ],
    )?;
    transaction.execute(
        "INSERT INTO events(instance_id, slot, kind, detail, created_at) \
             VALUES (?1, ?2, 'codex_offline_recovery', ?3, ?4)",
        params![slot.instance_id, slot.slot, thread, crate::db::now_unix()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
