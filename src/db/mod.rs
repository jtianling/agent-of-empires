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
use rusqlite::Connection;

/// Highest valid agent slot index. Slots are constrained to `0..=MAX_SLOT`
/// (at most four panes tracked per session).
pub const MAX_SLOT: i64 = 3;

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

    /// Read all durable slots for an instance, ordered by slot.
    pub fn read_slots_for_instance(&self, instance_id: &str) -> Result<Vec<AgentSlot>> {
        let mut stmt = self.conn.prepare(
            "SELECT instance_id, slot, agent, native_session_id, cwd, tmux_pane, \
             xats_identity_key, last_seen_at \
             FROM agent_slot WHERE instance_id = ?1 ORDER BY slot",
        )?;
        let rows = stmt.query_map([instance_id], |r| {
            Ok(AgentSlot {
                instance_id: r.get(0)?,
                slot: r.get(1)?,
                agent: r.get(2)?,
                native_session_id: r.get(3)?,
                cwd: r.get(4)?,
                tmux_pane: r.get(5)?,
                xats_identity_key: r.get(6)?,
                last_seen_at: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
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
    /// has none. Only adopted (non-primary) slots use it: the primary pane's key
    /// lives on the instance record next to its session id and resume token.
    pub xats_identity_key: String,
    pub last_seen_at: i64,
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
    for column in ["tmux_pane", "xats_identity_key"] {
        let has_column: bool = conn.query_row(
            "SELECT count(*) FROM pragma_table_info('agent_slot') WHERE name = ?1",
            [column],
            |r| r.get::<_, i64>(0).map(|n| n > 0),
        )?;
        if !has_column {
            conn.execute(
                &format!("ALTER TABLE agent_slot ADD COLUMN {column} TEXT NOT NULL DEFAULT ''"),
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

        assert!(column_exists(&store.conn, "agent_slot", "tmux_pane"));
        // The seeded row is preserved (column added, table not recreated) and
        // its backfilled `tmux_pane` defaults to the empty string.
        let slots = store.read_slots_for_instance("legacy").unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].native_session_id, "sess");
        assert_eq!(slots[0].tmux_pane, "");
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
}
