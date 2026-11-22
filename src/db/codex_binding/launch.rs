use anyhow::{ensure, Result};
use rusqlite::{params, TransactionBehavior};

use crate::db::{AgentSlot, Store};

impl Store {
    pub(crate) fn rebind_codex_launch_slot(
        &self,
        slot: &AgentSlot,
        pane: &str,
        cwd: &str,
    ) -> Result<bool> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE agent_slot SET tmux_pane=?1,cwd=?2, \
             xats_runtime_generation=xats_runtime_generation+1 \
             WHERE instance_id=?3 AND slot=?4 AND agent='codex' AND tmux_pane=?5 \
             AND xats_runtime_generation=?6 AND native_session_id=?7 AND xats_identity_key=?8 \
             AND xats_runtime_generation<?9 AND NOT EXISTS(SELECT 1 FROM codex_binding b \
                 WHERE b.instance_id=?3 AND b.slot=?4 AND b.state='restarting')",
            params![
                pane,
                cwd,
                slot.instance_id,
                slot.slot,
                slot.tmux_pane,
                slot.xats_runtime_generation,
                slot.native_session_id,
                slot.xats_identity_key,
                crate::db::MAX_XATS_RUNTIME_GENERATION
            ],
        )?;
        if changed != 1 {
            return Ok(false);
        }
        rebind_proof(&transaction, slot)?;
        transaction.commit()?;
        Ok(true)
    }

    pub(crate) fn persist_launched_codex_thread(
        &self,
        slot: &AgentSlot,
        thread: &str,
        launch: &str,
    ) -> Result<()> {
        uuid::Uuid::parse_str(thread)?;
        uuid::Uuid::parse_str(launch)?;
        ensure!(
            slot.agent == "codex" && slot.cross_agent_team,
            "Invalid Codex launcher slot"
        );
        ensure!(
            slot.native_session_id != thread,
            "Codex fresh launch reused its old UUID"
        );
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        ensure_unique(&transaction, slot, thread)?;
        let changed = transaction.execute(
            "UPDATE agent_slot SET native_session_id = ?1, \
             xats_runtime_generation = xats_runtime_generation + 1, last_seen_at = ?2 \
             WHERE instance_id = ?3 AND slot = ?4 AND agent = 'codex' \
             AND tmux_pane = ?5 AND xats_runtime_generation = ?6 \
             AND xats_runtime_generation < ?7 AND xats_identity_key = ?8 \
             AND native_session_id = ?9 AND NOT EXISTS(SELECT 1 FROM codex_binding b \
                 WHERE b.instance_id = ?3 AND b.slot = ?4 AND b.state = 'restarting')",
            params![
                thread,
                crate::db::now_unix(),
                slot.instance_id,
                slot.slot,
                slot.tmux_pane,
                slot.xats_runtime_generation,
                crate::db::MAX_XATS_RUNTIME_GENERATION,
                slot.xats_identity_key,
                slot.native_session_id
            ],
        )?;
        ensure!(
            changed == 1,
            "Codex slot changed before its UUID was persisted"
        );
        write_receipt(&transaction, slot, thread, launch)?;
        transaction.commit()?;
        Ok(())
    }
}

fn ensure_unique(
    transaction: &rusqlite::Transaction<'_>,
    slot: &AgentSlot,
    thread: &str,
) -> Result<()> {
    let duplicate: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_slot WHERE agent='codex' \
         AND native_session_id=?1 AND (instance_id!=?2 OR slot!=?3))",
        params![thread, slot.instance_id, slot.slot],
        |row| row.get(0),
    )?;
    ensure!(
        !duplicate,
        "Another Codex slot already owns the allocated UUID"
    );
    Ok(())
}

fn rebind_proof(transaction: &rusqlite::Transaction<'_>, slot: &AgentSlot) -> Result<()> {
    transaction.execute(
        "UPDATE codex_binding SET generation=?1,launch_id='',state='verified' \
         WHERE instance_id=?2 AND slot=?3 AND generation=?4 AND thread_id=?5 AND identity_key=?6",
        params![
            slot.xats_runtime_generation + 1,
            slot.instance_id,
            slot.slot,
            slot.xats_runtime_generation,
            slot.native_session_id,
            slot.xats_identity_key
        ],
    )?;
    transaction.execute(
        "DELETE FROM codex_binding WHERE instance_id=?1 AND slot=?2 AND generation!=?3",
        params![
            slot.instance_id,
            slot.slot,
            slot.xats_runtime_generation + 1
        ],
    )?;
    Ok(())
}

fn write_receipt(
    transaction: &rusqlite::Transaction<'_>,
    slot: &AgentSlot,
    thread: &str,
    launch: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO codex_binding(instance_id,slot,generation,identity_key,launch_id,thread_id,state) \
         VALUES (?1,?2,?3,?4,'',?5,'verified') ON CONFLICT(instance_id,slot) DO UPDATE SET \
         generation=excluded.generation,identity_key=excluded.identity_key,launch_id='', \
         thread_id=excluded.thread_id,state='verified'",
        params![slot.instance_id, slot.slot, slot.xats_runtime_generation + 1,
            slot.xats_identity_key, thread],
    )?;
    let detail = serde_json::json!({"thread_id":thread,"launch_id":launch,
        "previous_thread":slot.native_session_id})
    .to_string();
    transaction.execute(
        "INSERT INTO events(instance_id,slot,kind,detail,created_at) \
         VALUES (?1,?2,'codex_thread_created',?3,?4)",
        params![slot.instance_id, slot.slot, detail, crate::db::now_unix()],
    )?;
    Ok(())
}
