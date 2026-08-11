//! SQLite-backed durable store for per-pane agent session records.
//!
//! The store lives in `aoe.db` inside the active profile directory (next to
//! `sessions.json`). It records, per tmux pane, the agent's native session id
//! captured from hook stdin, snapshots those captures into durable per-slot
//! rows, and keeps an append-only event stream. It never stores conversation
//! content; agents keep their own transcripts.
//!
//! The schema is created by the migration system (see
//! `src/migrations/v006_agent_session_store.rs`). [`ensure_schema`] is the
//! single source of truth for the DDL and is idempotent (`CREATE TABLE IF NOT
//! EXISTS`), so both the migration and a defensive open path can apply it
//! safely.

pub mod codex_rollout;
pub mod reconcile;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction, TransactionBehavior};

/// Highest valid agent slot index. Slots are constrained to `0..=MAX_SLOT`
/// (at most four panes tracked per session).
pub const MAX_SLOT: i64 = 3;

/// Largest integer that round-trips exactly through JSON-based control planes.
pub const MAX_XATS_RUNTIME_GENERATION: i64 = 9_007_199_254_740_991;

/// Unbound extra-pane reservations become reclaimable after this lease.
const PENDING_SLOT_LEASE_SECS: i64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePreparationMode {
    Fresh,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSlotRuntime {
    pub generation: i64,
    pub native_session_id: String,
    pub xats_identity_key: String,
}

enum AvailableExtraSlot {
    Missing(i64),
    Stale {
        slot: i64,
        generation: i64,
        tmux_pane: String,
    },
}

/// How long an event row is kept. The stream is diagnostic only; nothing in the
/// codebase reads it back, so the window just has to cover a plausible debugging
/// session.
const EVENT_RETENTION_SECS: i64 = 7 * 24 * 60 * 60;

/// Most recent event rows kept per instance. Bounds a single busy instance so it
/// cannot crowd out the history of the others inside the retention window.
const EVENT_MAX_ROWS_PER_INSTANCE: i64 = 500;

/// Resolve the path to `aoe.db` for the given profile (next to `sessions.json`).
pub fn db_path(profile: &str) -> Result<PathBuf> {
    let dir = crate::session::ensure_profile_dir(profile)?;
    Ok(dir.join("aoe.db"))
}

/// A handle to the per-profile SQLite store.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the store for the active profile and apply
    /// pragmas. Assumes the schema has already been created by the migration
    /// system; callers that may run before migrations should call
    /// [`Store::open_with_schema`] instead.
    pub fn open(profile: &str) -> Result<Self> {
        let path = db_path(profile)?;
        Self::open_at(&path)
    }

    /// Like [`Store::open`], but also applies the schema idempotently. Used by
    /// paths that may run before the migration has executed (e.g. the
    /// `__record-pane` capture subcommand).
    ///
    /// A database that cannot be read at all is quarantined and recreated rather
    /// than failing: see [`open_with_schema_at`].
    pub fn open_with_schema(profile: &str) -> Result<Self> {
        let path = db_path(profile)?;
        let (store, _) = Self::open_with_schema_at(&path)?;
        Ok(store)
    }

    /// Open a store at `path`, applying the schema and pruning the event stream.
    ///
    /// When the file is corrupt or is not a database, it is moved aside under a
    /// timestamped name and an empty database is created in its place; the
    /// quarantined path is returned so the caller can surface it. The contents
    /// are derived state (captures re-appear within a tick, slots are re-assigned
    /// from live panes), so keeping the profile usable is worth more than the
    /// rows. The file is preserved rather than deleted: discarding gigabytes of a
    /// user's data is not a decision this code should make silently.
    ///
    /// Failures that are not corruption (permissions, locking, a missing
    /// directory) are returned to the caller unchanged.
    pub fn open_with_schema_at(path: &Path) -> Result<(Self, Option<PathBuf>)> {
        match Self::open_at(path).and_then(|store| {
            ensure_schema(&store.conn)?;
            Ok(store)
        }) {
            Ok(store) => Ok((store, None)),
            Err(e) if is_corruption(&e) => {
                let quarantined = quarantine_path(path);
                std::fs::rename(path, &quarantined).with_context(|| {
                    format!(
                        "moving unreadable store {} aside to {}",
                        path.display(),
                        quarantined.display()
                    )
                })?;
                // SQLite's sidecars belong to the file we just moved; leaving them
                // would make the fresh database inherit a stale journal.
                for suffix in ["-wal", "-shm"] {
                    let mut sidecar = path.as_os_str().to_os_string();
                    sidecar.push(suffix);
                    let _ = std::fs::remove_file(PathBuf::from(sidecar));
                }
                tracing::warn!(
                    "Unreadable session store at {} was moved to {} and recreated: {}",
                    path.display(),
                    quarantined.display(),
                    e
                );
                let store = Self::open_at(path)?;
                ensure_schema(&store.conn)?;
                Ok((store, Some(quarantined)))
            }
            Err(e) => Err(e),
        }
    }

    fn open_at(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening sqlite store at {}", path.display()))?;
        apply_pragmas(&conn)?;
        Ok(Self { conn })
    }

    /// Upsert the latest capture for a tmux pane.
    pub fn upsert_pane_live(
        &self,
        tmux_pane: &str,
        agent: &str,
        native_session_id: &str,
        cwd: &str,
        updated_at: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO pane_live (tmux_pane, agent, native_session_id, cwd, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(tmux_pane) DO UPDATE SET \
             agent = excluded.agent, \
             native_session_id = excluded.native_session_id, \
             cwd = excluded.cwd, \
             updated_at = excluded.updated_at",
            rusqlite::params![tmux_pane, agent, native_session_id, cwd, updated_at],
        )?;
        Ok(())
    }

    /// Read a single `pane_live` capture by tmux pane id, if present.
    pub fn read_pane_live(&self, tmux_pane: &str) -> Result<Option<PaneLive>> {
        let row = self
            .conn
            .query_row(
                "SELECT tmux_pane, agent, native_session_id, cwd, updated_at \
                 FROM pane_live WHERE tmux_pane = ?1",
                [tmux_pane],
                |r| {
                    Ok(PaneLive {
                        tmux_pane: r.get(0)?,
                        agent: r.get(1)?,
                        native_session_id: r.get(2)?,
                        cwd: r.get(3)?,
                        updated_at: r.get(4)?,
                    })
                },
            )
            .ok();
        Ok(row)
    }

    /// Every native session id any pane or slot currently claims. Used by the
    /// Codex rollout matcher so a conversation is never bound to two panes.
    pub fn claimed_native_session_ids(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT native_session_id FROM pane_live \
             UNION SELECT native_session_id FROM agent_slot",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = std::collections::HashSet::new();
        for r in rows {
            out.insert(r?);
        }
        Ok(out)
    }

    /// Return all `tmux_pane` keys currently present in `pane_live`.
    pub fn all_pane_live_keys(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT tmux_pane FROM pane_live")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Return `tmux_pane` keys captured strictly before `cutoff`.
    ///
    /// Used by orphan collection, which decides what is dead from a pane list
    /// read before it runs. A capture written after that read describes a pane
    /// the list could not have contained, so judging it against that list would
    /// delete a live pane's capture. Restricting collection to captures older
    /// than the read makes the two agree about the same moment. Seconds are the
    /// stored resolution, so the comparison is strict: a capture written in the
    /// same second as the read is treated as concurrent and left alone.
    pub fn pane_live_keys_before(&self, cutoff: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tmux_pane FROM pane_live WHERE updated_at < ?1")?;
        let rows = stmt.query_map([cutoff], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Delete a `pane_live` capture by tmux pane id.
    pub fn delete_pane_live(&self, tmux_pane: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM pane_live WHERE tmux_pane = ?1", [tmux_pane])?;
        Ok(())
    }

    /// Upsert a durable per-slot record. Rejects slots outside `0..=MAX_SLOT`.
    /// `tmux_pane` records which pane currently owns the slot so the reconciler
    /// can keep an already-tracked pane in its slot (sticky assignment).
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_agent_slot(
        &self,
        instance_id: &str,
        slot: i64,
        agent: &str,
        native_session_id: &str,
        cwd: &str,
        tmux_pane: &str,
        xats_identity_key: &str,
        last_seen_at: i64,
    ) -> Result<()> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        self.conn.execute(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, tmux_pane, xats_identity_key, \
              last_seen_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(instance_id, slot) DO UPDATE SET \
             agent = excluded.agent, \
             native_session_id = excluded.native_session_id, \
             cwd = excluded.cwd, \
             tmux_pane = excluded.tmux_pane, \
             xats_identity_key = excluded.xats_identity_key, \
             last_seen_at = excluded.last_seen_at",
            rusqlite::params![
                instance_id,
                slot,
                agent,
                native_session_id,
                cwd,
                tmux_pane,
                xats_identity_key,
                last_seen_at
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_agent_slot_config(
        &self,
        instance_id: &str,
        slot: i64,
        pane: &crate::session::PaneConfig,
        native_session_id: &str,
        tmux_pane: &str,
        xats_identity_key: &str,
        last_seen_at: i64,
    ) -> Result<()> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        pane.validate()?;
        let worktree_info = serialize_pane_worktree(pane.worktree.as_ref())?;
        self.conn.execute(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, tmux_pane, xats_identity_key, \
              yolo_mode, cross_agent_team, worktree_info, pane_config_version, last_seen_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11) \
             ON CONFLICT(instance_id, slot) DO UPDATE SET \
             agent = excluded.agent, native_session_id = excluded.native_session_id, \
             cwd = excluded.cwd, tmux_pane = excluded.tmux_pane, \
             xats_identity_key = excluded.xats_identity_key, \
             yolo_mode = excluded.yolo_mode, cross_agent_team = excluded.cross_agent_team, \
             worktree_info = excluded.worktree_info, pane_config_version = 1, \
             last_seen_at = excluded.last_seen_at",
            rusqlite::params![
                instance_id,
                slot,
                pane.tool,
                native_session_id,
                pane.working_dir,
                tmux_pane,
                xats_identity_key,
                pane.yolo_mode,
                pane.cross_agent_team,
                worktree_info,
                last_seen_at
            ],
        )?;
        Ok(())
    }

    /// Write the durable slot record of a pane AoE has just launched, before any
    /// capture exists for it.
    ///
    /// The native session id is empty because the pane has not reported a
    /// conversation yet; the reconciler fills it in from the first capture and
    /// carries this row's identity key forward. An empty native session id is a
    /// valid state, not an error: it degrades a relaunch of that pane to fresh.
    ///
    /// The conversation is always cleared, never carried over from whatever the
    /// slot held. The pane being recorded was created moments ago and cannot own
    /// a conversation yet, and a pane id is not a pane identity: tmux numbers
    /// panes from zero again after its server restarts, so a row naming `%1` may
    /// describe a pane from a previous server rather than this one. Matching on
    /// the id would hand this pane a dead pane's conversation.
    ///
    /// Clearing costs nothing that is not recoverable. The capture itself lives
    /// in `pane_live`, which a launch write never touches, so a capture that did
    /// land in the gap is written back by the next reconcile pass.
    #[allow(clippy::too_many_arguments)]
    pub fn record_launched_slot(
        &self,
        instance_id: &str,
        slot: i64,
        agent: &str,
        cwd: &str,
        tmux_pane: &str,
        xats_identity_key: &str,
        last_seen_at: i64,
    ) -> Result<()> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        self.conn.execute(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, tmux_pane, xats_identity_key, \
              last_seen_at) \
             VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7) \
             ON CONFLICT(instance_id, slot) DO UPDATE SET \
             agent = excluded.agent, \
             native_session_id = excluded.native_session_id, \
             cwd = excluded.cwd, \
             tmux_pane = excluded.tmux_pane, \
             xats_identity_key = excluded.xats_identity_key, \
             last_seen_at = excluded.last_seen_at",
            rusqlite::params![
                instance_id,
                slot,
                agent,
                cwd,
                tmux_pane,
                xats_identity_key,
                last_seen_at
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_launched_slot_config(
        &self,
        instance_id: &str,
        slot: i64,
        pane: &crate::session::PaneConfig,
        tmux_pane: &str,
        xats_identity_key: &str,
        last_seen_at: i64,
    ) -> Result<()> {
        self.upsert_agent_slot_config(
            instance_id,
            slot,
            pane,
            "",
            tmux_pane,
            xats_identity_key,
            last_seen_at,
        )
    }

    /// Snapshot a pane capture into its slot without disturbing the slot's
    /// identity key: a key already stored wins over the one passed in.
    ///
    /// A capture carries no key, so the caller has to supply one, and the only
    /// one it can supply is what it read before the write. Between that read and
    /// this write a launch can mint and store the pane's real key, and writing
    /// the read value back would erase it -- silently, and in the one direction
    /// nothing else repairs, because the pane is already running under the key
    /// that was erased. Letting the stored key win makes the read advisory.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_agent_slot_capture(
        &self,
        instance_id: &str,
        slot: i64,
        agent: &str,
        native_session_id: &str,
        cwd: &str,
        tmux_pane: &str,
        xats_identity_key: &str,
        last_seen_at: i64,
    ) -> Result<()> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        self.conn.execute(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, tmux_pane, xats_identity_key, \
              last_seen_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(instance_id, slot) DO UPDATE SET \
             agent = excluded.agent, \
             native_session_id = excluded.native_session_id, \
             cwd = excluded.cwd, \
             tmux_pane = excluded.tmux_pane, \
             xats_identity_key = CASE \
               WHEN agent_slot.xats_identity_key != '' \
               THEN agent_slot.xats_identity_key \
               ELSE excluded.xats_identity_key END, \
             last_seen_at = excluded.last_seen_at",
            rusqlite::params![
                instance_id,
                slot,
                agent,
                native_session_id,
                cwd,
                tmux_pane,
                xats_identity_key,
                last_seen_at
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_agent_slot_capture_config(
        &self,
        instance_id: &str,
        slot: i64,
        pane: &crate::session::PaneConfig,
        native_session_id: &str,
        tmux_pane: &str,
        xats_identity_key: &str,
        last_seen_at: i64,
    ) -> Result<()> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        pane.validate()?;
        let worktree_info = serialize_pane_worktree(pane.worktree.as_ref())?;
        self.conn.execute(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, tmux_pane, xats_identity_key, \
              yolo_mode, cross_agent_team, worktree_info, pane_config_version, last_seen_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11) \
             ON CONFLICT(instance_id, slot) DO UPDATE SET \
             agent = excluded.agent, native_session_id = excluded.native_session_id, \
             cwd = excluded.cwd, tmux_pane = excluded.tmux_pane, \
             yolo_mode = excluded.yolo_mode, \
             cross_agent_team = excluded.cross_agent_team, \
             xats_identity_key = CASE \
               WHEN agent_slot.xats_identity_key != '' THEN agent_slot.xats_identity_key \
               ELSE excluded.xats_identity_key END, \
             last_seen_at = excluded.last_seen_at",
            rusqlite::params![
                instance_id,
                slot,
                pane.tool,
                native_session_id,
                pane.working_dir,
                tmux_pane,
                xats_identity_key,
                pane.yolo_mode,
                pane.cross_agent_team,
                worktree_info,
                last_seen_at
            ],
        )?;
        Ok(())
    }

    /// Write a launch-time slot record only when the slot has none.
    ///
    /// Used for the primary pane, whose record is written as a side effect of
    /// launching a pane beside it. Reading the slot first and writing only when
    /// the read came back empty leaves a window in which a capture lands between
    /// the two and is then blanked; letting the conflict clause decide closes it.
    #[allow(clippy::too_many_arguments)]
    pub fn record_launched_slot_if_absent(
        &self,
        instance_id: &str,
        slot: i64,
        agent: &str,
        cwd: &str,
        tmux_pane: &str,
        xats_identity_key: &str,
        last_seen_at: i64,
    ) -> Result<()> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        self.conn.execute(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, tmux_pane, xats_identity_key, \
              last_seen_at) \
             VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7) \
             ON CONFLICT(instance_id, slot) DO NOTHING",
            rusqlite::params![
                instance_id,
                slot,
                agent,
                cwd,
                tmux_pane,
                xats_identity_key,
                last_seen_at
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_launched_slot_config_if_absent(
        &self,
        instance_id: &str,
        slot: i64,
        pane: &crate::session::PaneConfig,
        tmux_pane: &str,
        xats_identity_key: &str,
        last_seen_at: i64,
    ) -> Result<()> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        pane.validate()?;
        let worktree_info = serialize_pane_worktree(pane.worktree.as_ref())?;
        self.conn.execute(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, tmux_pane, xats_identity_key, \
              yolo_mode, cross_agent_team, worktree_info, pane_config_version, last_seen_at) \
             VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10) \
             ON CONFLICT(instance_id, slot) DO NOTHING",
            rusqlite::params![
                instance_id,
                slot,
                pane.tool,
                pane.working_dir,
                tmux_pane,
                xats_identity_key,
                pane.yolo_mode,
                pane.cross_agent_team,
                worktree_info,
                last_seen_at
            ],
        )?;
        Ok(())
    }

    /// Read all valid durable slots for an instance, ordered by slot.
    pub fn read_slots_for_instance(&self, instance_id: &str) -> Result<Vec<AgentSlot>> {
        Ok(self
            .read_slots_for_instance_with_diagnostics(instance_id)?
            .slots)
    }

    /// Atomically advance one slot's xats runtime generation. Fresh preparation
    /// clears that slot's conversation in the same statement; resume preserves it.
    pub fn prepare_opencode_runtime(
        &self,
        instance_id: &str,
        slot: i64,
        mode: RuntimePreparationMode,
    ) -> Result<PreparedSlotRuntime> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        let clear_session = mode == RuntimePreparationMode::Fresh;
        let transaction = self.conn.unchecked_transaction()?;
        let (prepared, old_tmux_pane) = transaction
            .query_row(
                "UPDATE agent_slot SET \
                 xats_runtime_generation = xats_runtime_generation + 1, \
                 native_session_id = CASE WHEN ?3 THEN '' ELSE native_session_id END \
                 WHERE instance_id = ?1 AND slot = ?2 \
                   AND xats_runtime_generation >= 0 \
                   AND xats_runtime_generation < ?4 \
                 RETURNING xats_runtime_generation, native_session_id, xats_identity_key, \
                           tmux_pane",
                rusqlite::params![
                    instance_id,
                    slot,
                    clear_session,
                    MAX_XATS_RUNTIME_GENERATION
                ],
                |row| {
                    Ok((
                        PreparedSlotRuntime {
                            generation: row.get(0)?,
                            native_session_id: row.get(1)?,
                            xats_identity_key: row.get(2)?,
                        },
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .with_context(|| {
                format!(
                    "preparing OpenCode runtime for instance '{}' slot {}",
                    instance_id, slot
                )
            })?;
        if clear_session && !old_tmux_pane.is_empty() {
            transaction.execute(
                "DELETE FROM pane_live WHERE tmux_pane = ?1",
                [old_tmux_pane],
            )?;
        }
        transaction.commit()?;
        Ok(prepared)
    }

    /// Provision a durable slot for an extra pane whose agent has an exact
    /// session runtime, before the pane itself exists.
    pub fn prepare_new_exact_session_slot(
        &self,
        instance_id: &str,
        pane: &crate::session::PaneConfig,
        identity_key: &str,
        live_pane_ids: &[String],
        live_snapshot_at: i64,
        last_seen_at: i64,
    ) -> Result<(i64, PreparedSlotRuntime)> {
        pane.validate()?;
        if crate::agents::exact_session_runtime(&pane.tool).is_none() {
            anyhow::bail!(
                "a new exact session slot requires a pane whose agent has one, not '{}'",
                pane.tool
            );
        }
        let worktree_info = serialize_pane_worktree(pane.worktree.as_ref())?;
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let available = select_available_extra_slot(
            &transaction,
            instance_id,
            live_pane_ids,
            live_snapshot_at,
            last_seen_at,
        )?;
        let (slot, generation) = write_new_exact_session_slot(
            &transaction,
            available,
            instance_id,
            pane,
            identity_key,
            &worktree_info,
            last_seen_at,
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

    /// Bind a pane id to a slot provisioned before tmux create/split without
    /// disturbing its prepared generation or exact session state.
    ///
    /// `native_session_id` is the conversation the caller believes the slot
    /// already holds, and completes the provisioning token the same way it does
    /// for a rollback: an owned-server caller passes an empty one because its
    /// runtime wrapper mints the conversation after the pane exists, while a
    /// shared-server caller passes the session AoE minted before launch.
    /// Assuming either shape here would silently match no row for the other.
    #[allow(clippy::too_many_arguments)]
    pub fn bind_prepared_slot_pane(
        &self,
        instance_id: &str,
        agent: &str,
        slot: i64,
        generation: i64,
        identity_key: &str,
        native_session_id: &str,
        tmux_pane: &str,
        last_seen_at: i64,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE agent_slot SET tmux_pane = ?1, last_seen_at = ?2 \
             WHERE instance_id = ?3 AND slot = ?4 AND agent = ?7 \
               AND xats_runtime_generation = ?5 AND xats_identity_key = ?6 \
               AND tmux_pane = '' AND native_session_id = ?8",
            rusqlite::params![
                tmux_pane,
                last_seen_at,
                instance_id,
                slot,
                generation,
                identity_key,
                agent,
                native_session_id,
            ],
        )?;
        if changed != 1 {
            anyhow::bail!(
                "prepared {} slot {} for instance '{}' was not found",
                agent,
                slot,
                instance_id
            );
        }
        Ok(())
    }

    /// Drop a slot provisioned for a pane that never launched.
    ///
    /// Scoped by the whole provisioning token -- agent, generation, key and the
    /// conversation the caller believes the slot holds -- and by the slot still
    /// being unbound, so a slot some other writer has advanced is never removed.
    /// An owned-server caller passes an empty conversation, because its runtime
    /// wrapper only records one onto a slot that already has a pane; a
    /// shared-server caller passes the session it minted before launch.
    pub fn rollback_unbound_exact_session_slot(
        &self,
        instance_id: &str,
        agent: &str,
        slot: i64,
        generation: i64,
        identity_key: &str,
        native_session_id: &str,
    ) -> Result<()> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        let changed = self.conn.execute(
            "DELETE FROM agent_slot \
             WHERE instance_id = ?1 AND slot = ?2 AND agent = ?5 \
               AND tmux_pane = '' AND native_session_id = ?6 \
               AND xats_runtime_generation = ?3 \
               AND xats_identity_key = ?4",
            rusqlite::params![
                instance_id,
                slot,
                generation,
                identity_key,
                agent,
                native_session_id
            ],
        )?;
        if changed != 1 {
            anyhow::bail!(
                "prepared {} slot {} for instance '{}' no longer matches rollback token",
                agent,
                slot,
                instance_id
            );
        }
        Ok(())
    }

    /// Record the exact OpenCode session only while the target slot still owns
    /// the generation that launched this runtime.
    #[allow(clippy::too_many_arguments)]
    pub fn record_opencode_runtime_session(
        &self,
        instance_id: &str,
        slot: i64,
        generation: i64,
        tmux_pane: &str,
        native_session_id: &str,
        cwd: &str,
        updated_at: i64,
    ) -> Result<bool> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        let transaction = self.conn.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE agent_slot SET native_session_id = ?1, tmux_pane = ?2, \
             last_seen_at = ?4 \
             WHERE instance_id = ?5 AND slot = ?6 AND agent = 'opencode' \
               AND xats_runtime_generation = ?7 AND cwd = ?3 AND tmux_pane = ?2",
            rusqlite::params![
                native_session_id,
                tmux_pane,
                cwd,
                updated_at,
                instance_id,
                slot,
                generation
            ],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO pane_live (tmux_pane, agent, native_session_id, cwd, updated_at) \
             VALUES (?1, 'opencode', ?2, ?3, ?4) \
             ON CONFLICT(tmux_pane) DO UPDATE SET \
             agent = excluded.agent, native_session_id = excluded.native_session_id, \
             cwd = excluded.cwd, updated_at = excluded.updated_at",
            rusqlite::params![tmux_pane, native_session_id, cwd, updated_at],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// Read slots and report records that could not safely be recovered.
    pub fn read_slots_for_instance_with_diagnostics(
        &self,
        instance_id: &str,
    ) -> Result<AgentSlotRead> {
        let (rows, unreadable) = self.read_raw_slots_for_instance(instance_id)?;
        let mut result = AgentSlotRead {
            skipped: unreadable,
            ..Default::default()
        };
        for row in rows {
            let slot = row.slot;
            match self.normalize_raw_slot(row) {
                Ok(record) => result.slots.push(record),
                Err(error) => {
                    tracing::warn!(
                        "Skipping invalid pane slot {} for instance '{}': {}",
                        slot,
                        instance_id,
                        error
                    );
                    result.skipped += 1;
                }
            }
        }
        Ok(result)
    }

    fn read_raw_slots_for_instance(&self, instance_id: &str) -> Result<(Vec<RawAgentSlot>, usize)> {
        let mut stmt = self.conn.prepare(
            "SELECT instance_id, slot, agent, native_session_id, cwd, tmux_pane, \
             xats_identity_key, xats_runtime_generation, yolo_mode, cross_agent_team, \
             worktree_info, last_seen_at \
             FROM agent_slot WHERE instance_id = ?1 ORDER BY slot",
        )?;
        let rows = stmt.query_map([instance_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, bool>(8)?,
                r.get::<_, bool>(9)?,
                r.get::<_, String>(10)?,
                r.get::<_, i64>(11)?,
            ))
        })?;
        let mut out = Vec::new();
        let mut unreadable = 0;
        for row in rows {
            let row = match row {
                Ok(row) => row,
                Err(error) => {
                    tracing::warn!(
                        "Skipping unreadable pane slot for instance '{}': {}",
                        instance_id,
                        error
                    );
                    unreadable += 1;
                    continue;
                }
            };
            let (
                row_instance_id,
                slot,
                agent,
                native_session_id,
                cwd,
                tmux_pane,
                xats_identity_key,
                xats_runtime_generation,
                yolo_mode,
                cross_agent_team,
                worktree_json,
                last_seen_at,
            ) = row;
            out.push(RawAgentSlot {
                instance_id: row_instance_id,
                slot,
                agent,
                native_session_id,
                cwd,
                tmux_pane,
                xats_identity_key,
                xats_runtime_generation,
                yolo_mode,
                cross_agent_team,
                worktree_json,
                last_seen_at,
            });
        }
        Ok((out, unreadable))
    }

    fn normalize_raw_slot(&self, row: RawAgentSlot) -> Result<AgentSlot> {
        let worktree_info = deserialize_pane_worktree(&row.worktree_json)?;
        let mut normalized = crate::session::PaneConfig::new(
            row.agent.clone(),
            row.cwd.clone(),
            row.yolo_mode,
            row.cross_agent_team,
        );
        normalized.worktree = worktree_info;
        normalized.validate()?;
        if normalized.yolo_mode != row.yolo_mode
            || normalized.cross_agent_team != row.cross_agent_team
        {
            self.conn.execute(
                "UPDATE agent_slot SET yolo_mode = ?1, cross_agent_team = ?2 \
                 WHERE instance_id = ?3 AND slot = ?4",
                rusqlite::params![
                    normalized.yolo_mode,
                    normalized.cross_agent_team,
                    row.instance_id.as_str(),
                    row.slot
                ],
            )?;
            tracing::warn!(
                "Normalized pane capability flags for slot {} in instance '{}'",
                row.slot,
                row.instance_id
            );
        }
        let record = AgentSlot {
            instance_id: row.instance_id,
            slot: row.slot,
            agent: row.agent,
            native_session_id: row.native_session_id,
            cwd: row.cwd,
            tmux_pane: row.tmux_pane,
            xats_identity_key: row.xats_identity_key,
            xats_runtime_generation: row.xats_runtime_generation,
            yolo_mode: normalized.yolo_mode,
            cross_agent_team: normalized.cross_agent_team,
            worktree_info: normalized.worktree,
            last_seen_at: row.last_seen_at,
        };
        Ok(record)
    }

    pub fn migrate_legacy_pane_configs(
        &self,
        instances: &[crate::session::Instance],
    ) -> Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        for instance in instances {
            let primary = instance.primary_pane_config();
            let primary_worktree = serialize_pane_worktree(primary.worktree.as_ref())?;
            let rows = {
                let mut statement = transaction.prepare(
                    "SELECT slot, agent, cwd, worktree_info, xats_identity_key \
                     FROM agent_slot \
                     WHERE instance_id = ?1 AND pane_config_version = 0",
                )?;
                let rows = statement
                    .query_map([instance.id.as_str()], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            for (slot, agent, cwd, stored_worktree, stored_key) in rows {
                let pane = crate::session::PaneConfig::new(
                    agent,
                    cwd,
                    primary.yolo_mode,
                    primary.cross_agent_team,
                );
                let worktree_info = if slot == 0 && stored_worktree.is_empty() {
                    primary_worktree.as_str()
                } else {
                    stored_worktree.as_str()
                };
                let identity_key = if slot == 0 && stored_key.is_empty() {
                    instance.xats_identity_key.as_deref().unwrap_or("")
                } else {
                    stored_key.as_str()
                };
                transaction.execute(
                    "UPDATE agent_slot SET \
                     yolo_mode = ?1, cross_agent_team = ?2, worktree_info = ?3, \
                     xats_identity_key = ?4, pane_config_version = 1 \
                     WHERE instance_id = ?5 AND slot = ?6 AND pane_config_version = 0",
                    rusqlite::params![
                        pane.yolo_mode,
                        pane.cross_agent_team,
                        worktree_info,
                        identity_key,
                        instance.id,
                        slot
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Remove all durable records for an instance (used on session deletion):
    /// its slots, its layout snapshot, and its events. Event rows outlive the
    /// session that produced them otherwise, and can never be read in context
    /// again once the instance is gone.
    pub fn delete_slots_for_instance(&self, instance_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM agent_slot WHERE instance_id = ?1",
            [instance_id],
        )?;
        self.conn
            .execute("DELETE FROM events WHERE instance_id = ?1", [instance_id])?;
        self.delete_layout_snapshot(instance_id)?;
        Ok(())
    }

    pub fn read_layout_snapshot(&self, instance_id: &str) -> Result<Option<LayoutSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT instance_id, window_layout, captured_at FROM instance_layout WHERE instance_id = ?1",
        )?;
        let mut rows = stmt.query([instance_id])?;
        Ok(match rows.next()? {
            Some(r) => Some(LayoutSnapshot {
                instance_id: r.get(0)?,
                window_layout: r.get(1)?,
                captured_at: r.get(2)?,
            }),
            None => None,
        })
    }

    /// Store a coherent layout, avoiding a database write when it is unchanged.
    pub fn upsert_layout_snapshot(
        &self,
        instance_id: &str,
        window_layout: &str,
        captured_at: i64,
    ) -> Result<bool> {
        let changed = match self.read_layout_snapshot(instance_id)? {
            Some(old) => old.window_layout != window_layout,
            None => true,
        };
        if changed {
            self.conn.execute(
                "INSERT INTO instance_layout (instance_id, window_layout, captured_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(instance_id) DO UPDATE SET
                 window_layout = excluded.window_layout,
                 captured_at = excluded.captured_at",
                rusqlite::params![instance_id, window_layout, captured_at],
            )?;
        }
        Ok(changed)
    }

    pub fn delete_layout_snapshot(&self, instance_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM instance_layout WHERE instance_id = ?1",
            [instance_id],
        )?;
        Ok(())
    }

    /// Append an event row. Events are append-only with a monotonic id.
    pub fn append_event(
        &self,
        instance_id: &str,
        slot: Option<i64>,
        kind: &str,
        detail: Option<&str>,
        created_at: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO events (instance_id, slot, kind, detail, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![instance_id, slot, kind, detail, created_at],
        )?;
        Ok(())
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AgentSlotRead {
    pub slots: Vec<AgentSlot>,
    pub skipped: usize,
}

struct RawAgentSlot {
    instance_id: String,
    slot: i64,
    agent: String,
    native_session_id: String,
    cwd: String,
    tmux_pane: String,
    xats_identity_key: String,
    xats_runtime_generation: i64,
    yolo_mode: bool,
    cross_agent_team: bool,
    worktree_json: String,
    last_seen_at: i64,
}

/// A volatile per-pane capture row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneLive {
    pub tmux_pane: String,
    pub agent: String,
    pub native_session_id: String,
    pub cwd: String,
    pub updated_at: i64,
}

/// A durable per-slot agent record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSlot {
    pub instance_id: String,
    pub slot: i64,
    pub agent: String,
    pub native_session_id: String,
    pub cwd: String,
    pub tmux_pane: String,
    /// Opaque xats identity key for the agent occupying this slot, empty when it
    /// has none. Every managed pane, including primary, owns its key through its
    /// durable slot record.
    pub xats_identity_key: String,
    pub xats_runtime_generation: i64,
    pub yolo_mode: bool,
    pub cross_agent_team: bool,
    pub worktree_info: Option<crate::session::PaneWorktreeInfo>,
    pub last_seen_at: i64,
}

impl AgentSlot {
    pub fn pane_config(&self) -> crate::session::PaneConfig {
        crate::session::PaneConfig {
            tool: self.agent.clone(),
            working_dir: self.cwd.clone(),
            yolo_mode: self.yolo_mode,
            cross_agent_team: self.cross_agent_team,
            worktree: self.worktree_info.clone(),
        }
    }
}

fn select_available_extra_slot(
    conn: &Connection,
    instance_id: &str,
    live_pane_ids: &[String],
    live_snapshot_at: i64,
    now: i64,
) -> Result<AvailableExtraSlot> {
    let mut statement = conn.prepare(
        "SELECT slot, xats_runtime_generation, tmux_pane, last_seen_at FROM agent_slot \
         WHERE instance_id = ?1 AND slot BETWEEN 1 AND ?2 ORDER BY slot",
    )?;
    let rows = statement
        .query_map(rusqlite::params![instance_id, MAX_SLOT], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for slot in 1..=MAX_SLOT {
        if !rows.iter().any(|(candidate, _, _, _)| *candidate == slot) {
            return Ok(AvailableExtraSlot::Missing(slot));
        }
    }
    let pending_cutoff = now.saturating_sub(PENDING_SLOT_LEASE_SECS);
    rows.into_iter()
        .find(|(_, generation, tmux_pane, last_seen_at)| {
            if *generation >= MAX_XATS_RUNTIME_GENERATION {
                return false;
            }
            if tmux_pane.is_empty() {
                return *last_seen_at <= pending_cutoff;
            }
            *last_seen_at < live_snapshot_at && !live_pane_ids.iter().any(|live| live == tmux_pane)
        })
        .map(
            |(slot, generation, tmux_pane, _)| AvailableExtraSlot::Stale {
                slot,
                generation,
                tmux_pane,
            },
        )
        .ok_or_else(|| anyhow::anyhow!("no available managed pane slot"))
}

fn write_new_exact_session_slot(
    transaction: &Transaction<'_>,
    available: AvailableExtraSlot,
    instance_id: &str,
    pane: &crate::session::PaneConfig,
    identity_key: &str,
    worktree_info: &str,
    last_seen_at: i64,
) -> Result<(i64, i64)> {
    match available {
        AvailableExtraSlot::Missing(slot) => insert_new_exact_session_slot(
            transaction,
            instance_id,
            slot,
            pane,
            identity_key,
            worktree_info,
            last_seen_at,
        ),
        AvailableExtraSlot::Stale {
            slot,
            generation,
            tmux_pane,
        } => replace_stale_exact_session_slot(
            transaction,
            instance_id,
            slot,
            generation,
            &tmux_pane,
            pane,
            identity_key,
            worktree_info,
            last_seen_at,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_new_exact_session_slot(
    transaction: &Transaction<'_>,
    instance_id: &str,
    slot: i64,
    pane: &crate::session::PaneConfig,
    identity_key: &str,
    worktree_info: &str,
    last_seen_at: i64,
) -> Result<(i64, i64)> {
    transaction.execute(
        "INSERT INTO agent_slot \
         (instance_id, slot, agent, native_session_id, cwd, tmux_pane, xats_identity_key, \
          xats_runtime_generation, yolo_mode, cross_agent_team, worktree_info, \
          pane_config_version, last_seen_at) \
         VALUES (?1, ?2, ?9, '', ?3, '', ?4, 1, ?5, ?6, ?7, 1, ?8)",
        rusqlite::params![
            instance_id,
            slot,
            pane.working_dir,
            identity_key,
            pane.yolo_mode,
            pane.cross_agent_team,
            worktree_info,
            last_seen_at,
            pane.tool,
        ],
    )?;
    Ok((slot, 1))
}

#[allow(clippy::too_many_arguments)]
fn replace_stale_exact_session_slot(
    transaction: &Transaction<'_>,
    instance_id: &str,
    slot: i64,
    generation: i64,
    tmux_pane: &str,
    pane: &crate::session::PaneConfig,
    identity_key: &str,
    worktree_info: &str,
    last_seen_at: i64,
) -> Result<(i64, i64)> {
    let next_generation = generation + 1;
    let changed = transaction.execute(
        "UPDATE agent_slot SET agent = ?12, native_session_id = '', cwd = ?1, \
         tmux_pane = '', xats_identity_key = ?2, xats_runtime_generation = ?3, \
         yolo_mode = ?4, cross_agent_team = ?5, worktree_info = ?6, \
         pane_config_version = 1, last_seen_at = ?7 \
         WHERE instance_id = ?8 AND slot = ?9 AND xats_runtime_generation = ?10 \
           AND tmux_pane = ?11",
        rusqlite::params![
            pane.working_dir,
            identity_key,
            next_generation,
            pane.yolo_mode,
            pane.cross_agent_team,
            worktree_info,
            last_seen_at,
            instance_id,
            slot,
            generation,
            tmux_pane,
            pane.tool,
        ],
    )?;
    if changed != 1 {
        anyhow::bail!("stale exact session slot reservation lost its compare-and-swap");
    }
    transaction.execute("DELETE FROM pane_live WHERE tmux_pane = ?1", [tmux_pane])?;
    Ok((slot, next_generation))
}

fn serialize_pane_worktree(info: Option<&crate::session::PaneWorktreeInfo>) -> Result<String> {
    match info {
        Some(info) if !info.is_empty() => serde_json::to_string(info).map_err(Into::into),
        _ => Ok(String::new()),
    }
}

fn deserialize_pane_worktree(value: &str) -> Result<Option<crate::session::PaneWorktreeInfo>> {
    if value.is_empty() {
        return Ok(None);
    }
    let info: crate::session::PaneWorktreeInfo = serde_json::from_str(value)?;
    if info.is_empty() {
        anyhow::bail!("empty pane worktree metadata");
    }
    Ok(Some(info))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutSnapshot {
    pub instance_id: String,
    pub window_layout: String,
    pub captured_at: i64,
}

fn apply_pragmas(conn: &Connection) -> Result<()> {
    // WAL mode tolerates concurrent hook-subprocess writers plus the reconciler;
    // a short busy timeout avoids spurious "database is locked" errors on tiny
    // upserts without blocking the agent.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 3000)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// Apply the store schema. Idempotent: safe to call repeatedly.
pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pane_live (
            tmux_pane          TEXT PRIMARY KEY,
            agent              TEXT NOT NULL,
            native_session_id  TEXT NOT NULL,
            cwd                TEXT NOT NULL,
            updated_at         INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS agent_slot (
            instance_id        TEXT NOT NULL,
            slot               INTEGER NOT NULL CHECK (slot >= 0 AND slot <= 3),
            agent              TEXT NOT NULL,
            native_session_id  TEXT NOT NULL,
            cwd                TEXT NOT NULL,
            tmux_pane          TEXT NOT NULL DEFAULT '',
            xats_identity_key  TEXT NOT NULL DEFAULT '',
            xats_runtime_generation INTEGER NOT NULL DEFAULT 0
                CHECK (xats_runtime_generation >= 0 AND xats_runtime_generation <= 9007199254740991),
            yolo_mode          INTEGER NOT NULL DEFAULT 0,
            cross_agent_team   INTEGER NOT NULL DEFAULT 0,
            worktree_info      TEXT NOT NULL DEFAULT '',
            pane_config_version INTEGER NOT NULL DEFAULT 0,
            last_seen_at       INTEGER NOT NULL,
            PRIMARY KEY (instance_id, slot)
        );

        CREATE TABLE IF NOT EXISTS events (
            id                 INTEGER PRIMARY KEY AUTOINCREMENT,
            instance_id        TEXT NOT NULL,
            slot               INTEGER,
            kind               TEXT NOT NULL,
            detail             TEXT,
            created_at         INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS instance_layout (
            instance_id        TEXT PRIMARY KEY,
            window_layout      TEXT NOT NULL,
            captured_at        INTEGER NOT NULL
        );",
    )?;
    backfill_agent_slot_columns(conn)?;
    prune_events(conn)?;
    Ok(())
}

/// Whether an error is SQLite reporting that the file cannot be read as a
/// database. Identified by SQLite's own result codes rather than by inspecting
/// the file, so the check cannot drift from what actually fails to open.
fn is_corruption(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase,
                    ..
                },
                _
            ))
        )
    })
}

/// Where an unreadable database is preserved. The timestamp keeps repeated
/// quarantines from overwriting each other.
fn quarantine_path(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".corrupt-{stamp}"));
    PathBuf::from(name)
}

/// Bound the event stream: drop rows outside the retention window, then keep only
/// the most recent rows per instance. Both bounds are needed -- age alone lets a
/// burst fill the window, and count alone lets an old, quiet database keep rows
/// forever.
///
/// This is a backstop. Events are appended on change rather than per poll tick,
/// so the table should stay small on its own; retention exists so a future caller
/// that appends carelessly cannot grow the file without limit.
fn prune_events(conn: &Connection) -> Result<()> {
    let cutoff = now_unix() - EVENT_RETENTION_SECS;
    let mut removed = conn.execute("DELETE FROM events WHERE created_at < ?1", [cutoff])?;

    removed += conn.execute(
        "DELETE FROM events WHERE id NOT IN (            SELECT id FROM events e2            WHERE e2.instance_id = events.instance_id            ORDER BY e2.id DESC LIMIT ?1          )",
        [EVENT_MAX_ROWS_PER_INSTANCE],
    )?;

    // Deleting rows frees pages inside the file without returning them to the
    // filesystem, so an already-oversized database would stay large. Only run it
    // when a prune actually removed rows, so a normal open does no extra work.
    if removed > 0 {
        conn.execute_batch("VACUUM")?;
    }
    Ok(())
}

/// Backfill `agent_slot` columns added after the table first shipped.
///
/// Because the table is created with `CREATE TABLE IF NOT EXISTS`, a database
/// created before a column was added keeps the older table shape, and
/// `upsert_agent_slot` (which writes every column) fails against it. Each column
/// is added when absent. Idempotent: a no-op once the column exists, so fresh and
/// already-healed databases are left untouched.
fn backfill_agent_slot_columns(conn: &Connection) -> Result<()> {
    for (column, definition) in [
        ("tmux_pane", "TEXT NOT NULL DEFAULT ''"),
        ("xats_identity_key", "TEXT NOT NULL DEFAULT ''"),
        (
            "xats_runtime_generation",
            "INTEGER NOT NULL DEFAULT 0 CHECK (xats_runtime_generation >= 0 AND xats_runtime_generation <= 9007199254740991)",
        ),
        ("yolo_mode", "INTEGER NOT NULL DEFAULT 0"),
        ("cross_agent_team", "INTEGER NOT NULL DEFAULT 0"),
        ("worktree_info", "TEXT NOT NULL DEFAULT ''"),
        ("pane_config_version", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let has_column: bool = conn.query_row(
            "SELECT count(*) FROM pragma_table_info('agent_slot') WHERE name = ?1",
            [column],
            |r| r.get::<_, i64>(0).map(|n| n > 0),
        )?;
        if !has_column {
            conn.execute(
                &format!("ALTER TABLE agent_slot ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
    }
    Ok(())
}

/// Apply the schema to the store for the given profile. Used by the migration.
pub fn create_schema_for_profile(profile: &str) -> Result<()> {
    let store = Store::open(profile)?;
    ensure_schema(&store.conn)?;
    Ok(())
}

/// Make a profile's store readable before anything depends on it, quarantining
/// and recreating it when it cannot be opened at all. Returns the path the
/// unreadable file was preserved at, so the caller can tell the user.
///
/// Called at startup ahead of the migrations, which apply the schema and abort on
/// error: without this, one profile's corrupt database makes AoE refuse to launch
/// in that profile. The routine callers of the store stay tolerant of an
/// unopenable store on their own.
pub fn ensure_store_readable(profile: &str) -> Result<Option<PathBuf>> {
    let path = db_path(profile)?;
    let (_, quarantined) = Store::open_with_schema_at(&path)?;
    Ok(quarantined)
}

/// Purge a deleted session's durable and volatile records from the store.
///
/// Removes the instance's `agent_slot` rows and any `pane_live` rows for
/// `pane_ids` (the session's panes, captured before its tmux session is
/// killed). Event rows are retained for history. Best-effort and silent on
/// error so it never blocks the delete path.
pub fn purge_session_records(profile: &str, instance_id: &str, pane_ids: &[String]) {
    let store = match Store::open_with_schema(profile) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("purge_session_records: cannot open store: {}", e);
            return;
        }
    };
    if let Err(e) = store.delete_slots_for_instance(instance_id) {
        tracing::debug!("purge_session_records: delete slots failed: {}", e);
    }
    for pane in pane_ids {
        if let Err(e) = store.delete_pane_live(pane) {
            tracing::debug!(
                "purge_session_records: delete pane_live {} failed: {}",
                pane,
                e
            );
        }
    }
}

/// Current unix timestamp in seconds, for `updated_at`/`last_seen_at`.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (TempDir, Store) {
        let tmp = TempDir::new().unwrap();
        let store = Store::open_at(&tmp.path().join("aoe.db")).unwrap();
        ensure_schema(&store.conn).unwrap();
        (tmp, store)
    }

    #[test]
    fn pane_live_upserts_by_pane() {
        let (_tmp, store) = temp_store();
        store
            .upsert_pane_live("%5", "claude", "first", "/tmp", 1)
            .unwrap();
        store
            .upsert_pane_live("%5", "claude", "second", "/tmp", 2)
            .unwrap();

        let row = store.read_pane_live("%5").unwrap().unwrap();
        assert_eq!(row.native_session_id, "second");
        assert_eq!(store.all_pane_live_keys().unwrap().len(), 1);
    }

    #[test]
    fn agent_slot_upserts_by_instance_and_slot() {
        let (_tmp, store) = temp_store();
        store
            .upsert_agent_slot("inst", 1, "claude", "old", "/tmp", "%1", "", 1)
            .unwrap();
        store
            .upsert_agent_slot("inst", 1, "claude", "new", "/tmp", "%1", "", 2)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].native_session_id, "new");
        assert_eq!(slots[0].tmux_pane, "%1");
    }

    #[test]
    fn agent_slot_range_rejected_by_api() {
        let (_tmp, store) = temp_store();
        assert!(store
            .upsert_agent_slot("inst", 4, "claude", "s", "/tmp", "%1", "", 1)
            .is_err());
        assert!(store
            .upsert_agent_slot("inst", -1, "claude", "s", "/tmp", "%1", "", 1)
            .is_err());
    }

    #[test]
    fn agent_slot_range_rejected_by_check_constraint() {
        // Direct SQL bypasses the API guard; the CHECK constraint must still
        // reject out-of-range slots (the e2e test writes raw SQL).
        let (_tmp, store) = temp_store();
        let err = store.conn.execute(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
             VALUES ('x', 4, 'claude', 's', '/tmp', 1)",
            [],
        );
        assert!(err.is_err());
    }

    #[test]
    fn events_are_append_only_monotonic() {
        let (_tmp, store) = temp_store();
        store
            .append_event("inst", Some(0), "status", Some("running"), 1)
            .unwrap();
        store
            .append_event("inst", Some(0), "capture", Some("sess"), 2)
            .unwrap();

        let count: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM events WHERE instance_id = 'inst'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);

        let max_id: i64 = store
            .conn
            .query_row("SELECT max(id) FROM events", [], |r| r.get(0))
            .unwrap();
        assert!(max_id >= 2);
    }

    #[test]
    fn delete_slots_for_instance_removes_rows() {
        let (_tmp, store) = temp_store();
        store
            .upsert_agent_slot("inst", 0, "claude", "s", "/tmp", "%1", "", 1)
            .unwrap();
        store
            .upsert_agent_slot("other", 0, "claude", "s", "/tmp", "%2", "", 1)
            .unwrap();
        store
            .upsert_layout_snapshot("inst", "0000,1x1,0,0,1", 1)
            .unwrap();

        store.delete_slots_for_instance("inst").unwrap();
        assert!(store.read_slots_for_instance("inst").unwrap().is_empty());
        assert!(store.read_layout_snapshot("inst").unwrap().is_none());
        assert_eq!(store.read_slots_for_instance("other").unwrap().len(), 1);
    }

    #[test]
    fn layout_snapshot_upserts_only_when_changed_and_deletes() {
        let (_tmp, store) = temp_store();
        assert!(store
            .upsert_layout_snapshot("inst", "aaaa,layout", 1)
            .unwrap());
        assert!(!store
            .upsert_layout_snapshot("inst", "aaaa,layout", 2)
            .unwrap());
        assert_eq!(
            store
                .read_layout_snapshot("inst")
                .unwrap()
                .unwrap()
                .captured_at,
            1
        );
        assert!(store.upsert_layout_snapshot("inst", "bbbb,new", 3).unwrap());
        store.delete_layout_snapshot("inst").unwrap();
        assert!(store.read_layout_snapshot("inst").unwrap().is_none());
    }

    #[test]
    fn ensure_schema_is_idempotent() {
        let (_tmp, store) = temp_store();
        store
            .upsert_agent_slot("keep", 0, "claude", "s", "/tmp", "%1", "", 1)
            .unwrap();
        // Re-applying the schema must not drop rows.
        ensure_schema(&store.conn).unwrap();
        ensure_schema(&store.conn).unwrap();
        assert_eq!(store.read_slots_for_instance("keep").unwrap().len(), 1);
    }

    /// Create a legacy `agent_slot` (6 columns, no `tmux_pane`) and seed a row,
    /// mirroring a database created before the column was added to the DDL.
    fn legacy_store_with_seeded_row() -> (TempDir, Store) {
        let tmp = TempDir::new().unwrap();
        let store = Store::open_at(&tmp.path().join("aoe.db")).unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TABLE agent_slot (
                    instance_id        TEXT NOT NULL,
                    slot               INTEGER NOT NULL CHECK (slot >= 0 AND slot <= 3),
                    agent              TEXT NOT NULL,
                    native_session_id  TEXT NOT NULL,
                    cwd                TEXT NOT NULL,
                    last_seen_at       INTEGER NOT NULL,
                    PRIMARY KEY (instance_id, slot)
                );",
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO agent_slot \
                 (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
                 VALUES ('legacy', 0, 'claude', 'sess', '/tmp', 1)",
                [],
            )
            .unwrap();
        (tmp, store)
    }

    fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
        conn.query_row(
            "SELECT count(*) FROM pragma_table_info(?1) WHERE name = ?2",
            rusqlite::params![table, column],
            |r| r.get::<_, i64>(0).map(|n| n > 0),
        )
        .unwrap()
    }

    #[test]
    fn ensure_schema_backfills_legacy_agent_slot_column() {
        let (_tmp, store) = legacy_store_with_seeded_row();
        assert!(!column_exists(&store.conn, "agent_slot", "tmux_pane"));

        ensure_schema(&store.conn).unwrap();

        for column in [
            "tmux_pane",
            "xats_identity_key",
            "xats_runtime_generation",
            "yolo_mode",
            "cross_agent_team",
            "worktree_info",
            "pane_config_version",
        ] {
            assert!(column_exists(&store.conn, "agent_slot", column));
        }
        // The seeded row is preserved (column added, table not recreated) and
        // its backfilled `tmux_pane` defaults to the empty string.
        let slots = store.read_slots_for_instance("legacy").unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].native_session_id, "sess");
        assert_eq!(slots[0].tmux_pane, "");
        assert_eq!(slots[0].xats_runtime_generation, 0);
    }

    #[test]
    fn upsert_agent_slot_succeeds_after_backfill() {
        let (_tmp, store) = legacy_store_with_seeded_row();
        ensure_schema(&store.conn).unwrap();

        // Before the fix this failed with "no such column: tmux_pane".
        store
            .upsert_agent_slot("legacy", 1, "claude", "new", "/tmp", "%9", "", 2)
            .unwrap();

        let slots = store.read_slots_for_instance("legacy").unwrap();
        assert_eq!(slots.len(), 2);
        let added = slots.iter().find(|s| s.slot == 1).unwrap();
        assert_eq!(added.tmux_pane, "%9");
    }

    #[test]
    fn backfill_is_idempotent() {
        // Re-running over a freshly-healed legacy database does not error.
        let (_tmp, store) = legacy_store_with_seeded_row();
        ensure_schema(&store.conn).unwrap();
        ensure_schema(&store.conn).unwrap();
        assert!(column_exists(&store.conn, "agent_slot", "tmux_pane"));

        // A fresh database already has the column; the backfill must not try to
        // add a duplicate column.
        let (_fresh_tmp, fresh) = temp_store();
        ensure_schema(&fresh.conn).unwrap();
        assert!(column_exists(&fresh.conn, "agent_slot", "tmux_pane"));
    }

    #[test]
    fn backfill_heals_xats_identity_key_column() {
        let (_tmp, store) = legacy_store_with_seeded_row();
        assert!(!column_exists(
            &store.conn,
            "agent_slot",
            "xats_identity_key"
        ));

        ensure_schema(&store.conn).unwrap();
        assert!(column_exists(
            &store.conn,
            "agent_slot",
            "xats_identity_key"
        ));

        // Writing every column now succeeds against the healed legacy table.
        store
            .upsert_agent_slot("legacy", 2, "claude", "s", "/tmp", "%3", "key-2", 3)
            .unwrap();
        let slots = store.read_slots_for_instance("legacy").unwrap();
        let added = slots.iter().find(|s| s.slot == 2).unwrap();
        assert_eq!(added.xats_identity_key, "key-2");
    }

    #[test]
    fn prune_drops_events_outside_the_retention_window() {
        let (_tmp, store) = temp_store();
        ensure_schema(&store.conn).unwrap();
        let now = now_unix();

        store
            .append_event(
                "inst",
                Some(0),
                "capture",
                None,
                now - EVENT_RETENTION_SECS - 1,
            )
            .unwrap();
        store
            .append_event("inst", Some(0), "capture", None, now)
            .unwrap();

        prune_events(&store.conn).unwrap();

        let kept: i64 = store
            .conn
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 1, "only the row inside the window survives");
    }

    #[test]
    fn prune_caps_rows_per_instance_without_touching_others() {
        let (_tmp, store) = temp_store();
        ensure_schema(&store.conn).unwrap();
        let now = now_unix();

        for _ in 0..(EVENT_MAX_ROWS_PER_INSTANCE + 25) {
            store
                .append_event("busy", Some(0), "capture", None, now)
                .unwrap();
        }
        store
            .append_event("quiet", Some(0), "adopt", None, now)
            .unwrap();

        prune_events(&store.conn).unwrap();

        let busy: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM events WHERE instance_id = 'busy'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let quiet: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM events WHERE instance_id = 'quiet'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(busy, EVENT_MAX_ROWS_PER_INSTANCE);
        assert_eq!(quiet, 1, "a busy instance must not evict another's history");
    }

    #[test]
    fn prune_leaves_a_store_within_its_bounds_alone() {
        let (_tmp, store) = temp_store();
        ensure_schema(&store.conn).unwrap();
        let now = now_unix();
        store
            .append_event("inst", Some(0), "adopt", None, now)
            .unwrap();

        prune_events(&store.conn).unwrap();

        let kept: i64 = store
            .conn
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 1);
    }

    #[test]
    fn deleting_a_session_purges_its_events_only() {
        let (_tmp, store) = temp_store();
        ensure_schema(&store.conn).unwrap();
        let now = now_unix();
        store
            .append_event("gone", Some(0), "adopt", None, now)
            .unwrap();
        store
            .append_event("kept", Some(0), "adopt", None, now)
            .unwrap();
        store
            .upsert_agent_slot("gone", 0, "claude", "s", "/tmp", "%1", "", now)
            .unwrap();

        store.delete_slots_for_instance("gone").unwrap();

        let gone: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM events WHERE instance_id = 'gone'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let kept: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM events WHERE instance_id = 'kept'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(gone, 0, "a deleted session must not leave events behind");
        assert_eq!(kept, 1);
    }

    #[test]
    fn corrupt_database_is_quarantined_and_recreated() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("aoe.db");
        // A valid SQLite header followed by garbage: opens, then fails to read.
        let mut bytes = b"SQLite format 3\0".to_vec();
        bytes.extend(std::iter::repeat(0xAB).take(8192));
        std::fs::write(&path, &bytes).unwrap();

        let (store, quarantined) = Store::open_with_schema_at(&path).unwrap();
        let quarantined = quarantined.expect("corrupt file should be quarantined");

        assert!(
            quarantined.exists(),
            "the unreadable file must be preserved"
        );
        assert!(path.exists(), "a fresh database must take its place");
        store
            .upsert_agent_slot("inst", 0, "claude", "s", "/tmp", "%1", "", 1)
            .unwrap();
    }

    #[test]
    fn a_file_that_is_not_a_database_is_quarantined() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("aoe.db");
        std::fs::write(&path, b"this is not a database at all").unwrap();

        let (_store, quarantined) = Store::open_with_schema_at(&path).unwrap();
        assert!(quarantined.expect("should be quarantined").exists());
    }

    #[test]
    fn a_healthy_database_is_not_quarantined() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("aoe.db");

        let (_store, first) = Store::open_with_schema_at(&path).unwrap();
        assert!(first.is_none());
        let (_store, second) = Store::open_with_schema_at(&path).unwrap();
        assert!(
            second.is_none(),
            "reopening a healthy store changes nothing"
        );
    }

    #[test]
    fn a_launched_slot_round_trips_with_an_empty_native_session_id() {
        // The pane has not reported a conversation yet. An empty native session
        // id is a valid state, and the key it carries is the point of the row.
        let (_tmp, store) = temp_store();

        store
            .record_launched_slot("inst", 1, "codex", "/tmp", "%7", "launched-key", 9)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].native_session_id, "");
        assert_eq!(slots[0].tmux_pane, "%7");
        assert_eq!(slots[0].agent, "codex");
        assert_eq!(slots[0].xats_identity_key, "launched-key");
    }

    #[test]
    fn a_launched_slot_write_never_carries_a_conversation_over() {
        // Even when the row names the same pane id. tmux numbers panes from zero
        // again after its server restarts, so id equality does not establish that
        // the recorded conversation belongs to the pane being launched, and the
        // capture is re-applied from `pane_live` by the next reconcile anyway.
        let (_tmp, store) = temp_store();

        store
            .upsert_agent_slot("inst", 1, "codex", "conv-old", "/tmp", "%7", "key-old", 1)
            .unwrap();
        store
            .record_launched_slot("inst", 1, "codex", "/tmp", "%7", "key-new", 9)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].native_session_id, "");
        assert_eq!(slots[0].xats_identity_key, "key-new");
    }

    #[test]
    fn a_launched_slot_write_drops_a_dead_pane_s_conversation_and_key() {
        // Reclaiming the slot for a different pane must not hand the new pane the
        // old pane's conversation or its identity.
        let (_tmp, store) = temp_store();

        store
            .upsert_agent_slot("inst", 1, "codex", "conv-old", "/tmp", "%7", "key-old", 1)
            .unwrap();
        store
            .record_launched_slot("inst", 1, "codex", "/tmp", "%9", "key-new", 9)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].tmux_pane, "%9");
        assert_eq!(slots[0].native_session_id, "");
        assert_eq!(slots[0].xats_identity_key, "key-new");
    }

    #[test]
    fn a_capture_write_keeps_a_key_stored_after_its_caller_read() {
        // The caller can only pass the key it read before writing. A launch that
        // stores the pane's real key in that gap must not be undone by the
        // write-back of that stale read.
        let (_tmp, store) = temp_store();

        store
            .record_launched_slot("inst", 1, "codex", "/tmp", "%7", "minted-key", 5)
            .unwrap();
        store
            .upsert_agent_slot_capture("inst", 1, "codex", "conv-1", "/tmp", "%7", "", 9)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].native_session_id, "conv-1");
        assert_eq!(slots[0].xats_identity_key, "minted-key");
    }

    #[test]
    fn a_capture_write_supplies_the_key_when_the_slot_has_none() {
        let (_tmp, store) = temp_store();

        store
            .upsert_agent_slot_capture("inst", 1, "codex", "conv-1", "/tmp", "%7", "carried", 9)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].xats_identity_key, "carried");
    }

    #[test]
    fn recording_a_slot_if_absent_leaves_an_existing_row_alone() {
        // The primary pane's row is written as a side effect of launching a pane
        // beside it, so it must never overwrite a row that already carries one.
        let (_tmp, store) = temp_store();

        store
            .upsert_agent_slot("inst", 0, "claude", "conv-0", "/tmp", "%1", "key-0", 1)
            .unwrap();
        store
            .record_launched_slot_if_absent("inst", 0, "claude", "/tmp", "%1", "", 9)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].native_session_id, "conv-0");
        assert_eq!(slots[0].xats_identity_key, "key-0");
    }

    #[test]
    fn recording_a_slot_if_absent_writes_when_the_slot_is_empty() {
        let (_tmp, store) = temp_store();

        store
            .record_launched_slot_if_absent("inst", 0, "claude", "/tmp", "%1", "", 9)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].tmux_pane, "%1");
        assert_eq!(slots[0].native_session_id, "");
    }

    #[test]
    fn identity_key_round_trips_and_defaults_to_empty() {
        let (_tmp, store) = temp_store();
        ensure_schema(&store.conn).unwrap();

        store
            .upsert_agent_slot("inst", 0, "claude", "s", "/tmp", "%1", "", 1)
            .unwrap();
        store
            .upsert_agent_slot("inst", 1, "claude", "s", "/tmp", "%2", "key-1", 1)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].xats_identity_key, "");
        assert_eq!(slots[1].xats_identity_key, "key-1");
    }

    #[test]
    fn pane_config_round_trips_flags_and_worktree_metadata() {
        let (_tmp, store) = temp_store();
        let pane = crate::session::PaneConfig {
            tool: "codex".to_string(),
            working_dir: "/tmp/right".to_string(),
            yolo_mode: true,
            cross_agent_team: true,
            worktree: Some(crate::session::PaneWorktreeInfo {
                worktree_path: Some("/tmp/right".to_string()),
                worktree: Some(crate::session::WorktreeInfo {
                    branch: "right-branch".to_string(),
                    main_repo_path: "/tmp/repo".to_string(),
                    managed_by_aoe: true,
                    created_at: chrono::Utc::now(),
                    cleanup_on_delete: true,
                }),
                workspace: None,
            }),
        };

        store
            .record_launched_slot_config("inst", 1, &pane, "%2", "right-key", 9)
            .unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].pane_config(), pane);
        assert_eq!(slots[0].xats_identity_key, "right-key");
    }

    #[test]
    fn capture_tool_change_normalizes_flags_and_preserves_worktree() {
        let (_tmp, store) = temp_store();
        let mut launched = crate::session::PaneConfig::new("codex", "/tmp/right", true, true);
        launched.worktree = Some(crate::session::PaneWorktreeInfo {
            worktree_path: Some("/tmp/owned".to_string()),
            worktree: Some(crate::session::WorktreeInfo {
                branch: "right-branch".to_string(),
                main_repo_path: "/tmp/repo".to_string(),
                managed_by_aoe: true,
                created_at: chrono::Utc::now(),
                cleanup_on_delete: true,
            }),
            workspace: None,
        });
        store
            .record_launched_slot_config("inst", 1, &launched, "%2", "right-key", 1)
            .unwrap();

        let captured = crate::session::PaneConfig::new("shell", "/tmp/runtime", true, true);
        store
            .upsert_agent_slot_capture_config("inst", 1, &captured, "native", "%2", "", 2)
            .unwrap();

        let slot = store.read_slots_for_instance("inst").unwrap().remove(0);
        assert_eq!(slot.agent, "shell");
        assert!(!slot.yolo_mode);
        assert!(!slot.cross_agent_team);
        assert_eq!(
            slot.worktree_info.and_then(|info| info.worktree_path),
            Some("/tmp/owned".to_string())
        );
    }

    #[test]
    fn invalid_persisted_worktree_metadata_is_skipped_at_read_boundary() {
        let (_tmp, store) = temp_store();
        store
            .conn
            .execute(
                "INSERT INTO agent_slot \
                 (instance_id, slot, agent, native_session_id, cwd, worktree_info, last_seen_at) \
                 VALUES ('inst', 0, 'claude', '', '/tmp', '{invalid', 1)",
                [],
            )
            .unwrap();
        store
            .upsert_agent_slot("inst", 1, "codex", "right", "/tmp", "%2", "", 2)
            .unwrap();

        let read = store
            .read_slots_for_instance_with_diagnostics("inst")
            .unwrap();
        assert_eq!(read.slots.len(), 1);
        assert_eq!(read.slots[0].slot, 1);
        assert_eq!(read.skipped, 1);
    }

    #[test]
    fn read_normalizes_and_repairs_persisted_capability_flags() {
        let (_tmp, store) = temp_store();
        store
            .conn
            .execute(
                "INSERT INTO agent_slot \
                 (instance_id, slot, agent, native_session_id, cwd, yolo_mode, \
                  cross_agent_team, pane_config_version, last_seen_at) \
                 VALUES ('inst', 0, 'shell', '', '/tmp', 1, 1, 1, 1)",
                [],
            )
            .unwrap();

        let read = store
            .read_slots_for_instance_with_diagnostics("inst")
            .unwrap();
        assert_eq!(read.skipped, 0);
        assert_eq!(read.slots.len(), 1);
        assert!(!read.slots[0].yolo_mode);
        assert!(!read.slots[0].cross_agent_team);

        let persisted: (bool, bool) = store
            .conn
            .query_row(
                "SELECT yolo_mode, cross_agent_team FROM agent_slot \
                 WHERE instance_id = 'inst' AND slot = 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(persisted, (false, false));
    }

    #[test]
    fn legacy_pane_migration_is_idempotent_and_inherits_shared_values() {
        let (_tmp, store) = temp_store();
        store
            .upsert_agent_slot("inst", 0, "shell", "left", "/tmp/left", "%1", "", 1)
            .unwrap();
        store
            .upsert_agent_slot("inst", 1, "codex", "right", "/tmp/right", "%2", "", 1)
            .unwrap();
        store
            .upsert_agent_slot("inst", 2, "shell", "", "/tmp/shell", "%3", "", 1)
            .unwrap();
        let mut instance = crate::session::Instance::new("test", "/tmp/left");
        instance.id = "inst".to_string();
        instance.yolo_mode = true;
        instance.cross_agent_team = true;
        instance.xats_identity_key = Some("left-key".to_string());
        instance.worktree_info = Some(crate::session::WorktreeInfo {
            branch: "left-branch".to_string(),
            main_repo_path: "/tmp/repo".to_string(),
            managed_by_aoe: true,
            created_at: chrono::Utc::now(),
            cleanup_on_delete: true,
        });
        instance.primary_pane = crate::session::PaneConfig::default();
        instance.hydrate_legacy_primary_pane();

        store
            .migrate_legacy_pane_configs(&[instance.clone()])
            .unwrap();
        store.migrate_legacy_pane_configs(&[instance]).unwrap();

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].agent, "shell");
        assert!(!slots[0].yolo_mode);
        assert!(!slots[0].cross_agent_team);
        assert!(slots[1].yolo_mode);
        assert!(slots[1].cross_agent_team);
        assert!(!slots[2].yolo_mode);
        assert!(!slots[2].cross_agent_team);
        assert_eq!(slots[0].xats_identity_key, "left-key");
        assert_eq!(
            slots[0]
                .worktree_info
                .as_ref()
                .and_then(|info| info.worktree.as_ref())
                .map(|info| info.branch.as_str()),
            Some("left-branch")
        );
        assert_eq!(
            slots[0]
                .worktree_info
                .as_ref()
                .and_then(|info| info.worktree_path.as_deref()),
            Some("/tmp/left")
        );
        assert!(slots[1].xats_identity_key.is_empty());
        assert!(slots[1].worktree_info.is_none());
    }

    #[test]
    fn opencode_runtime_preparation_is_per_slot_and_mode_aware() {
        let (_tmp, store) = temp_store();
        for slot in 0..=2 {
            store
                .upsert_agent_slot(
                    "inst",
                    slot,
                    "opencode",
                    &format!("ses_{slot}"),
                    "/tmp",
                    &format!("%{slot}"),
                    &format!("key-{slot}"),
                    1,
                )
                .unwrap();
        }

        let resumed = store
            .prepare_opencode_runtime("inst", 1, RuntimePreparationMode::Resume)
            .unwrap();
        assert_eq!(resumed.generation, 1);
        assert_eq!(resumed.native_session_id, "ses_1");
        assert_eq!(resumed.xats_identity_key, "key-1");

        store
            .upsert_pane_live("%1", "opencode", "ses_1", "/tmp", 2)
            .unwrap();
        let fresh = store
            .prepare_opencode_runtime("inst", 1, RuntimePreparationMode::Fresh)
            .unwrap();
        assert_eq!(fresh.generation, 2);
        assert!(fresh.native_session_id.is_empty());
        assert_eq!(fresh.xats_identity_key, "key-1");
        assert!(store.read_pane_live("%1").unwrap().is_none());

        let slots = store.read_slots_for_instance("inst").unwrap();
        assert_eq!(slots[0].xats_runtime_generation, 0);
        assert_eq!(slots[1].xats_runtime_generation, 2);
        assert_eq!(slots[2].xats_runtime_generation, 0);
        assert_eq!(slots[0].native_session_id, "ses_0");
        assert_eq!(slots[2].native_session_id, "ses_2");
    }

    #[test]
    fn new_opencode_runtime_reservations_do_not_reuse_pending_slots() {
        let (_tmp, store) = temp_store();
        let pane = crate::session::PaneConfig::new("opencode", "/tmp", false, false);

        let (first_slot, first) = store
            .prepare_new_exact_session_slot("inst", &pane, "", &[], 1, 1)
            .unwrap();
        let (second_slot, second) = store
            .prepare_new_exact_session_slot("inst", &pane, "", &[], 2, 2)
            .unwrap();
        let (third_slot, third) = store
            .prepare_new_exact_session_slot("inst", &pane, "", &[], 3, 3)
            .unwrap();

        assert_eq!((first_slot, first.generation), (1, 1));
        assert_eq!((second_slot, second.generation), (2, 1));
        assert_eq!((third_slot, third.generation), (3, 1));
        assert!(store
            .prepare_new_exact_session_slot(
                "inst",
                &pane,
                "",
                &[],
                PENDING_SLOT_LEASE_SECS,
                PENDING_SLOT_LEASE_SECS,
            )
            .is_err());
    }

    #[test]
    fn new_opencode_runtime_reclaims_an_expired_pending_slot() {
        let (_tmp, store) = temp_store();
        let pane = crate::session::PaneConfig::new("opencode", "/tmp", false, true);
        for key in ["key-a", "key-b", "key-c"] {
            store
                .prepare_new_exact_session_slot("inst", &pane, key, &[], 1, 1)
                .unwrap();
        }

        let (slot, prepared) = store
            .prepare_new_exact_session_slot(
                "inst",
                &pane,
                "replacement-key",
                &[],
                PENDING_SLOT_LEASE_SECS + 1,
                PENDING_SLOT_LEASE_SECS + 1,
            )
            .unwrap();

        assert_eq!((slot, prepared.generation), (1, 2));
        let row = &store.read_slots_for_instance("inst").unwrap()[0];
        assert!(row.tmux_pane.is_empty());
        assert_eq!(row.xats_identity_key, "replacement-key");
    }

    #[test]
    fn new_opencode_runtime_reclaims_only_a_non_live_bound_slot() {
        let (_tmp, store) = temp_store();
        for (slot, tmux_pane) in [(1, "%dead"), (2, "%live-2"), (3, "%live-3")] {
            store
                .upsert_agent_slot(
                    "inst",
                    slot,
                    "claude",
                    "old-session",
                    "/old",
                    tmux_pane,
                    "old-key",
                    1,
                )
                .unwrap();
        }
        store
            .conn
            .execute(
                "UPDATE agent_slot SET xats_runtime_generation = 7 \
                 WHERE instance_id = 'inst' AND slot = 1",
                [],
            )
            .unwrap();
        store
            .upsert_pane_live("%dead", "claude", "old-session", "/old", 1)
            .unwrap();
        let pane = crate::session::PaneConfig::new("opencode", "/new", false, true);
        let live = ["%live-2".to_string(), "%live-3".to_string()];

        let (slot, prepared) = store
            .prepare_new_exact_session_slot("inst", &pane, "new-key", &live, 2, 2)
            .unwrap();

        assert_eq!((slot, prepared.generation), (1, 8));
        let row = &store.read_slots_for_instance("inst").unwrap()[0];
        assert_eq!(row.agent, "opencode");
        assert!(row.native_session_id.is_empty());
        assert!(row.tmux_pane.is_empty());
        assert_eq!(row.xats_identity_key, "new-key");
        assert!(store.read_pane_live("%dead").unwrap().is_none());
    }

    #[test]
    fn stale_live_snapshot_cannot_reclaim_a_pane_bound_after_capture() {
        let (tmp, snapshot_store) = temp_store();
        let binder = Store::open_at(&tmp.path().join("aoe.db")).unwrap();
        let pane = crate::session::PaneConfig::new("opencode", "/tmp", false, true);
        let (slot, prepared) = snapshot_store
            .prepare_new_exact_session_slot("inst", &pane, "new-key", &[], 1, 1)
            .unwrap();
        for (slot, tmux_pane) in [(2, "%live-2"), (3, "%live-3")] {
            snapshot_store
                .upsert_agent_slot(
                    "inst", slot, "opencode", "ses_live", "/tmp", tmux_pane, "live-key", 1,
                )
                .unwrap();
        }

        let live_snapshot_at = 10;
        binder
            .bind_prepared_slot_pane(
                "inst",
                "opencode",
                slot,
                prepared.generation,
                "new-key",
                "",
                "%new",
                11,
            )
            .unwrap();
        let live = ["%live-2".to_string(), "%live-3".to_string()];

        assert!(snapshot_store
            .prepare_new_exact_session_slot(
                "inst",
                &pane,
                "replacement-key",
                &live,
                live_snapshot_at,
                12,
            )
            .is_err());
        let row = &snapshot_store.read_slots_for_instance("inst").unwrap()[0];
        assert_eq!(row.tmux_pane, "%new");
        assert_eq!(row.xats_runtime_generation, prepared.generation);
        assert_eq!(row.xats_identity_key, "new-key");
    }

    #[test]
    fn concurrent_opencode_runtime_reservations_get_distinct_slots() {
        use std::sync::{Arc, Barrier};

        let (tmp, first_store) = temp_store();
        let second_store = Store::open_at(&tmp.path().join("aoe.db")).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let prepare = |store: Store, key: &'static str, barrier: Arc<Barrier>| {
            std::thread::spawn(move || {
                let pane = crate::session::PaneConfig::new("opencode", "/tmp", false, true);
                barrier.wait();
                store
                    .prepare_new_exact_session_slot("inst", &pane, key, &[], 1, 1)
                    .unwrap()
                    .0
            })
        };
        let first = prepare(first_store, "key-a", Arc::clone(&barrier));
        let second = prepare(second_store, "key-b", Arc::clone(&barrier));
        barrier.wait();

        let mut slots = [first.join().unwrap(), second.join().unwrap()];
        slots.sort();
        assert_eq!(slots, [1, 2]);
    }

    #[test]
    fn prepared_opencode_bind_requires_exact_reservation_token() {
        let (_tmp, store) = temp_store();
        let pane = crate::session::PaneConfig::new("opencode", "/tmp", false, true);
        let (slot, prepared) = store
            .prepare_new_exact_session_slot("inst", &pane, "key", &[], 1, 1)
            .unwrap();

        assert!(store
            .bind_prepared_slot_pane(
                "inst",
                "opencode",
                slot,
                prepared.generation + 1,
                "key",
                "",
                "%1",
                2,
            )
            .is_err());
        assert!(store
            .bind_prepared_slot_pane(
                "inst",
                "opencode",
                slot,
                prepared.generation,
                "other-key",
                "",
                "%1",
                2,
            )
            .is_err());
        store
            .bind_prepared_slot_pane(
                "inst",
                "opencode",
                slot,
                prepared.generation,
                "key",
                "",
                "%1",
                2,
            )
            .unwrap();
        assert_eq!(
            store.read_slots_for_instance("inst").unwrap()[0].tmux_pane,
            "%1"
        );
    }

    /// A shared-server slot carries the session AoE minted before the split, so
    /// binding it must match on that conversation rather than on the empty one an
    /// owned-server slot still has. Assuming either shape matched no row for the
    /// other, which surfaced as an extra pane that launched and then reported its
    /// identity key as unrecorded.
    #[test]
    fn prepared_shared_server_bind_matches_the_session_minted_before_launch() {
        let (_tmp, store) = temp_store();
        let pane = crate::session::PaneConfig::new("kimi", "/tmp", false, true);
        let (slot, prepared) = store
            .prepare_new_exact_session_slot("inst", &pane, "key", &[], 1, 1)
            .unwrap();
        store
            .upsert_agent_slot_config("inst", slot, &pane, "session_abc", "", "key", 1)
            .unwrap();

        assert!(store
            .bind_prepared_slot_pane(
                "inst",
                "kimi",
                slot,
                prepared.generation,
                "key",
                "",
                "%1",
                2
            )
            .is_err());
        assert!(store
            .bind_prepared_slot_pane(
                "inst",
                "opencode",
                slot,
                prepared.generation,
                "key",
                "session_abc",
                "%1",
                2
            )
            .is_err());

        store
            .bind_prepared_slot_pane(
                "inst",
                "kimi",
                slot,
                prepared.generation,
                "key",
                "session_abc",
                "%1",
                2,
            )
            .unwrap();
        let row = &store.read_slots_for_instance("inst").unwrap()[0];
        assert_eq!(row.tmux_pane, "%1");
        assert_eq!(row.native_session_id, "session_abc");
    }

    #[test]
    fn ordinary_upserts_and_capture_preserve_runtime_generation() {
        let (_tmp, store) = temp_store();
        store
            .upsert_agent_slot("inst", 0, "opencode", "ses_old", "/tmp", "%1", "key", 1)
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE agent_slot SET xats_runtime_generation = 8 \
                 WHERE instance_id = 'inst' AND slot = 0",
                [],
            )
            .unwrap();

        store
            .upsert_agent_slot_capture("inst", 0, "opencode", "ses_new", "/tmp", "%1", "", 2)
            .unwrap();
        store
            .upsert_agent_slot("inst", 0, "opencode", "ses_new", "/tmp", "%1", "key", 3)
            .unwrap();

        let slot = &store.read_slots_for_instance("inst").unwrap()[0];
        assert_eq!(slot.xats_runtime_generation, 8);
        assert_eq!(slot.native_session_id, "ses_new");
    }

    #[test]
    fn exact_opencode_capture_rejects_stale_generation() {
        let (_tmp, store) = temp_store();
        store
            .upsert_agent_slot("inst", 0, "opencode", "ses_old", "/tmp", "%1", "key", 1)
            .unwrap();
        let prepared = store
            .prepare_opencode_runtime("inst", 0, RuntimePreparationMode::Fresh)
            .unwrap();

        assert!(!store
            .record_opencode_runtime_session(
                "inst",
                0,
                prepared.generation - 1,
                "%1",
                "ses_stale",
                "/tmp",
                2,
            )
            .unwrap());
        assert!(store.read_pane_live("%1").unwrap().is_none());
        assert!(store.read_slots_for_instance("inst").unwrap()[0]
            .native_session_id
            .is_empty());

        assert!(!store
            .record_opencode_runtime_session(
                "inst",
                0,
                prepared.generation,
                "%2",
                "ses_wrong_pane",
                "/tmp",
                3,
            )
            .unwrap());
        assert!(store
            .record_opencode_runtime_session(
                "inst",
                0,
                prepared.generation,
                "%1",
                "ses_current",
                "/tmp",
                4,
            )
            .unwrap());
        let slot = &store.read_slots_for_instance("inst").unwrap()[0];
        assert_eq!(slot.native_session_id, "ses_current");
        assert_eq!(slot.tmux_pane, "%1");
    }

    #[test]
    fn prepared_opencode_slot_rollback_requires_exact_unbound_generation() {
        let (_tmp, store) = temp_store();
        store
            .upsert_agent_slot("inst", 1, "opencode", "", "/tmp", "", "key", 1)
            .unwrap();
        let prepared = store
            .prepare_opencode_runtime("inst", 1, RuntimePreparationMode::Fresh)
            .unwrap();

        assert!(store
            .rollback_unbound_exact_session_slot(
                "inst",
                "opencode",
                1,
                prepared.generation - 1,
                "key",
                ""
            )
            .is_err());
        assert_eq!(store.read_slots_for_instance("inst").unwrap().len(), 1);

        store
            .conn
            .execute(
                "UPDATE agent_slot SET native_session_id = 'ses_ready' \
                 WHERE instance_id = 'inst' AND slot = 1",
                [],
            )
            .unwrap();
        assert!(store
            .rollback_unbound_exact_session_slot(
                "inst",
                "opencode",
                1,
                prepared.generation,
                "key",
                ""
            )
            .is_err());
        store
            .conn
            .execute(
                "UPDATE agent_slot SET native_session_id = '' \
                 WHERE instance_id = 'inst' AND slot = 1",
                [],
            )
            .unwrap();

        store
            .rollback_unbound_exact_session_slot(
                "inst",
                "opencode",
                1,
                prepared.generation,
                "key",
                "",
            )
            .unwrap();
        assert!(store.read_slots_for_instance("inst").unwrap().is_empty());
    }

    #[test]
    fn schema_healing_does_not_reset_existing_generation() {
        let (_tmp, store) = temp_store();
        store
            .upsert_agent_slot("inst", 0, "opencode", "ses_a", "/tmp", "%1", "key", 1)
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE agent_slot SET xats_runtime_generation = 17 \
                 WHERE instance_id = 'inst' AND slot = 0",
                [],
            )
            .unwrap();
        ensure_schema(&store.conn).unwrap();
        ensure_schema(&store.conn).unwrap();
        assert_eq!(
            store.read_slots_for_instance("inst").unwrap()[0].xats_runtime_generation,
            17
        );
    }
}
