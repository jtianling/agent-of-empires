use anyhow::{Context, Result};
use serde::Deserialize;

use crate::db::Store;

#[derive(Deserialize)]
struct Failure {
    pane: String,
    generation: i64,
    thread: String,
    error: String,
}

impl Store {
    pub(crate) fn primary_codex_launch_failure(&self, instance: &str) -> Result<Option<String>> {
        let Some(slot) = self
            .read_slots_for_instance(instance)?
            .into_iter()
            .find(|slot| slot.slot == 0 && slot.agent == "codex" && slot.cross_agent_team)
        else {
            return Ok(None);
        };
        let mut statement = self.conn.prepare(
            "SELECT detail FROM events WHERE instance_id = ?1 AND slot = 0 \
             AND kind = 'codex_launch_failed' ORDER BY id DESC LIMIT 8",
        )?;
        let rows = statement.query_map([instance], |row| row.get::<_, String>(0))?;
        for row in rows {
            let failure: Failure =
                serde_json::from_str(&row?).context("Invalid recorded Codex launch failure")?;
            if failure.pane == slot.tmux_pane
                && failure.generation == slot.xats_runtime_generation
                && failure.thread == slot.native_session_id
            {
                return Ok(Some(failure.error));
            }
        }
        Ok(None)
    }
}
