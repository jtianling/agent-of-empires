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

pub mod reconcile;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Highest valid agent slot index. Slots are constrained to `0..=MAX_SLOT`
/// (at most four panes tracked per session).
pub const MAX_SLOT: i64 = 3;

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
    pub fn open_with_schema(profile: &str) -> Result<Self> {
        let store = Self::open(profile)?;
        ensure_schema(&store.conn)?;
        Ok(store)
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
        last_seen_at: i64,
    ) -> Result<()> {
        if !(0..=MAX_SLOT).contains(&slot) {
            anyhow::bail!("slot {} out of range 0..={}", slot, MAX_SLOT);
        }
        self.conn.execute(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, tmux_pane, last_seen_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(instance_id, slot) DO UPDATE SET \
             agent = excluded.agent, \
             native_session_id = excluded.native_session_id, \
             cwd = excluded.cwd, \
             tmux_pane = excluded.tmux_pane, \
             last_seen_at = excluded.last_seen_at",
            rusqlite::params![
                instance_id,
                slot,
                agent,
                native_session_id,
                cwd,
                tmux_pane,
                last_seen_at
            ],
        )?;
        Ok(())
    }

    /// Read all durable slots for an instance, ordered by slot.
    pub fn read_slots_for_instance(&self, instance_id: &str) -> Result<Vec<AgentSlot>> {
        let mut stmt = self.conn.prepare(
            "SELECT instance_id, slot, agent, native_session_id, cwd, tmux_pane, last_seen_at \
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
                last_seen_at: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Remove all durable slots for an instance (used on session deletion).
    pub fn delete_slots_for_instance(&self, instance_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM agent_slot WHERE instance_id = ?1",
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
    pub last_seen_at: i64,
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
        );",
    )?;
    backfill_agent_slot_tmux_pane(conn)?;
    Ok(())
}

/// Backfill the `agent_slot.tmux_pane` column on legacy databases.
///
/// `agent_slot` shipped before `tmux_pane` was added to its DDL. Because the
/// table is created with `CREATE TABLE IF NOT EXISTS`, those legacy databases
/// keep a 6-column table and `upsert_agent_slot` (which writes `tmux_pane`)
/// fails. This adds the column when it is absent. Idempotent: a no-op once the
/// column exists, so it leaves fresh and already-healed databases untouched.
fn backfill_agent_slot_tmux_pane(conn: &Connection) -> Result<()> {
    let has_column: bool = conn.query_row(
        "SELECT count(*) FROM pragma_table_info('agent_slot') WHERE name = 'tmux_pane'",
        [],
        |r| r.get::<_, i64>(0).map(|n| n > 0),
    )?;
    if !has_column {
        conn.execute(
            "ALTER TABLE agent_slot ADD COLUMN tmux_pane TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    Ok(())
}

/// Apply the schema to the store for the given profile. Used by the migration.
pub fn create_schema_for_profile(profile: &str) -> Result<()> {
    let store = Store::open(profile)?;
    ensure_schema(&store.conn)?;
    Ok(())
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
            .upsert_agent_slot("inst", 1, "claude", "old", "/tmp", "%1", 1)
            .unwrap();
        store
            .upsert_agent_slot("inst", 1, "claude", "new", "/tmp", "%1", 2)
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
            .upsert_agent_slot("inst", 4, "claude", "s", "/tmp", "%1", 1)
            .is_err());
        assert!(store
            .upsert_agent_slot("inst", -1, "claude", "s", "/tmp", "%1", 1)
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
            .upsert_agent_slot("inst", 0, "claude", "s", "/tmp", "%1", 1)
            .unwrap();
        store
            .upsert_agent_slot("other", 0, "claude", "s", "/tmp", "%2", 1)
            .unwrap();

        store.delete_slots_for_instance("inst").unwrap();
        assert!(store.read_slots_for_instance("inst").unwrap().is_empty());
        assert_eq!(store.read_slots_for_instance("other").unwrap().len(), 1);
    }

    #[test]
    fn ensure_schema_is_idempotent() {
        let (_tmp, store) = temp_store();
        store
            .upsert_agent_slot("keep", 0, "claude", "s", "/tmp", "%1", 1)
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
            .upsert_agent_slot("legacy", 1, "claude", "new", "/tmp", "%9", 2)
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
}
