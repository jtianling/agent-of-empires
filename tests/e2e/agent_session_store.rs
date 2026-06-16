//! RED e2e tests for the `agent-session-store` capability.
//!
//! These exercise the real `aoe` binary end-to-end: the SQLite store
//! (`aoe.db`) is expected to be created under the active profile directory by
//! the migration system, expose the `pane_live`, `agent_slot`, and `events`
//! tables, and be cleaned up when a session is deleted.
//!
//! All tests are RED until the feature lands: there is no `aoe.db`, no
//! migration creating it, and no `aoe __record-pane` capture subcommand yet.
//!
//! The store is observed from outside the binary via the `sqlite3` CLI (the
//! real on-disk artifact), so the assertions never depend on in-process state.

use std::path::PathBuf;
use std::process::Command;

use serial_test::serial;

use crate::harness::TuiTestHarness;

/// Skip the calling test if the `sqlite3` CLI is not installed. The store is a
/// real `aoe.db` file; we observe it from outside the binary.
macro_rules! require_sqlite3 {
    () => {
        if !sqlite3_available() {
            eprintln!("Skipping test: sqlite3 CLI not available");
            return;
        }
    };
}

fn sqlite3_available() -> bool {
    Command::new("sqlite3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Absolute path to `aoe.db` inside the default profile directory of the
/// harness's isolated HOME.
fn db_path(h: &TuiTestHarness) -> PathBuf {
    let profile_dir = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default")
    } else {
        h.home_path().join(".agent-of-empires/profiles/default")
    };
    profile_dir.join("aoe.db")
}

/// Run a SQL statement against `aoe.db` via the `sqlite3` CLI and return stdout.
fn sqlite_query(db: &std::path::Path, sql: &str) -> String {
    let output = Command::new("sqlite3")
        .arg("-cmd")
        .arg(".timeout 5000")
        .arg(db)
        .arg(sql)
        .output()
        .expect("failed to run sqlite3");
    assert!(
        output.status.success(),
        "sqlite3 query failed for {:?}: {}\nstdout: {}",
        sql,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Run a SQL statement that may fail (e.g. a constraint violation) and return
/// `(success, combined_output)`.
fn sqlite_try(db: &std::path::Path, sql: &str) -> (bool, String) {
    let output = Command::new("sqlite3")
        .arg("-cmd")
        .arg(".timeout 5000")
        .arg(db)
        .arg(sql)
        .output()
        .expect("failed to run sqlite3");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// Register a session and start its tmux process so the store is initialized
/// for a real, managed instance. Returns the session's instance id.
fn add_and_start(h: &TuiTestHarness, title: &str) -> String {
    let project = h.project_path();
    let add = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        title,
        "--cmd-override",
        "sh",
    ]);
    assert!(
        add.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let start = h.run_cli_in_tmux(&["session", "start", title]);
    assert!(
        start.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    instance_id_for_title(h, title)
}

fn instance_id_for_title(h: &TuiTestHarness, title: &str) -> String {
    let sessions_path = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    };
    let content = std::fs::read_to_string(&sessions_path).expect("read sessions.json");
    let sessions: serde_json::Value = serde_json::from_str(&content).expect("parse sessions.json");
    sessions
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|s| s["title"].as_str() == Some(title))
        .and_then(|s| s["id"].as_str())
        .unwrap_or_else(|| panic!("missing session {}", title))
        .to_string()
}

// ---------------------------------------------------------------------------
// Requirement: SQLite store is created under the active profile directory
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn store_database_created_on_first_run() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("store_db_created");
    // Starting a managed session must trigger store creation via migrations.
    add_and_start(&h, "Store Created");

    let db = db_path(&h);
    assert!(
        db.exists(),
        "expected aoe.db to be created under the profile dir at {}",
        db.display()
    );

    // All required tables must exist before any read or write.
    let tables = sqlite_query(
        &db,
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;",
    );
    for required in ["agent_slot", "events", "pane_live"] {
        assert!(
            tables.lines().any(|t| t == required),
            "expected table {} to exist in aoe.db; found tables:\n{}",
            required,
            tables
        );
    }
}

#[test]
#[serial]
fn store_migration_is_idempotent() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("store_idempotent");
    add_and_start(&h, "Store Idempotent");

    let db = db_path(&h);
    assert!(db.exists(), "aoe.db should exist after first run");

    // Seed a durable row, then re-run the binary (which re-runs migrations).
    sqlite_query(
        &db,
        "INSERT INTO agent_slot \
         (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
         VALUES ('mig-keep', 0, 'claude', 'sess-keep', '/tmp', 0);",
    );

    // A second invocation must not error and must not drop existing rows.
    let list = h.run_cli(&["list"]);
    assert!(
        list.status.success(),
        "second aoe invocation failed (migration not idempotent?): {}",
        String::from_utf8_lossy(&list.stderr)
    );

    let count = sqlite_query(
        &db,
        "SELECT count(*) FROM agent_slot WHERE instance_id='mig-keep';",
    );
    assert_eq!(
        count, "1",
        "idempotent migration must preserve existing rows, got count={}",
        count
    );
}

#[test]
#[serial]
fn store_path_is_profile_scoped() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("store_profile_scoped");
    add_and_start(&h, "Profile Scoped");

    let default_db = db_path(&h);
    assert!(default_db.exists(), "default profile aoe.db should exist");

    // The store must live under the active profile directory, never at the
    // app root shared across profiles.
    let app_root_db = if cfg!(target_os = "linux") {
        h.home_path().join(".config/agent-of-empires/aoe.db")
    } else {
        h.home_path().join(".agent-of-empires/aoe.db")
    };
    assert!(
        !app_root_db.exists(),
        "aoe.db must be profile-scoped, not at the shared app root {}",
        app_root_db.display()
    );
}

// ---------------------------------------------------------------------------
// Requirement: Volatile per-pane capture table
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn pane_live_upserts_by_tmux_pane() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("pane_live_upsert");
    add_and_start(&h, "Pane Live Upsert");
    let db = db_path(&h);

    // Two captures for the same pane with different native_session_id: the row
    // must reflect the most recent capture, and there must be exactly one row.
    sqlite_query(
        &db,
        "INSERT INTO pane_live (tmux_pane, agent, native_session_id, cwd, updated_at) \
         VALUES ('%5', 'claude', 'first-sess', '/tmp', 1) \
         ON CONFLICT(tmux_pane) DO UPDATE SET \
         native_session_id=excluded.native_session_id, updated_at=excluded.updated_at;",
    );
    sqlite_query(
        &db,
        "INSERT INTO pane_live (tmux_pane, agent, native_session_id, cwd, updated_at) \
         VALUES ('%5', 'claude', 'second-sess', '/tmp', 2) \
         ON CONFLICT(tmux_pane) DO UPDATE SET \
         native_session_id=excluded.native_session_id, updated_at=excluded.updated_at;",
    );

    let count = sqlite_query(&db, "SELECT count(*) FROM pane_live WHERE tmux_pane='%5';");
    assert_eq!(count, "1", "exactly one pane_live row per tmux_pane");

    let latest = sqlite_query(
        &db,
        "SELECT native_session_id FROM pane_live WHERE tmux_pane='%5';",
    );
    assert_eq!(
        latest, "second-sess",
        "pane_live must reflect the most recent capture"
    );
}

// ---------------------------------------------------------------------------
// Requirement: Durable per-slot agent record
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn agent_slot_upserts_by_instance_and_slot() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("agent_slot_upsert");
    add_and_start(&h, "Agent Slot Upsert");
    let db = db_path(&h);

    sqlite_query(
        &db,
        "INSERT INTO agent_slot \
         (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
         VALUES ('inst-a', 1, 'claude', 'sess-old', '/tmp', 1) \
         ON CONFLICT(instance_id, slot) DO UPDATE SET \
         native_session_id=excluded.native_session_id, last_seen_at=excluded.last_seen_at;",
    );
    sqlite_query(
        &db,
        "INSERT INTO agent_slot \
         (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
         VALUES ('inst-a', 1, 'claude', 'sess-new', '/tmp', 2) \
         ON CONFLICT(instance_id, slot) DO UPDATE SET \
         native_session_id=excluded.native_session_id, last_seen_at=excluded.last_seen_at;",
    );

    let count = sqlite_query(
        &db,
        "SELECT count(*) FROM agent_slot WHERE instance_id='inst-a' AND slot=1;",
    );
    assert_eq!(
        count, "1",
        "no duplicate row for the same (instance_id, slot)"
    );

    let value = sqlite_query(
        &db,
        "SELECT native_session_id FROM agent_slot WHERE instance_id='inst-a' AND slot=1;",
    );
    assert_eq!(value, "sess-new", "existing row updated in place");
}

#[test]
#[serial]
fn agent_slot_range_is_enforced() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("agent_slot_range");
    add_and_start(&h, "Agent Slot Range");
    let db = db_path(&h);

    // Slot must be constrained to 0..3; a write outside that range is rejected.
    let (ok_high, out_high) = sqlite_try(
        &db,
        "INSERT INTO agent_slot \
         (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
         VALUES ('inst-bad', 4, 'claude', 'sess', '/tmp', 1);",
    );
    assert!(
        !ok_high,
        "slot=4 must be rejected by a CHECK constraint; sqlite3 output:\n{}",
        out_high
    );

    let (ok_neg, out_neg) = sqlite_try(
        &db,
        "INSERT INTO agent_slot \
         (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
         VALUES ('inst-bad', -1, 'claude', 'sess', '/tmp', 1);",
    );
    assert!(
        !ok_neg,
        "slot=-1 must be rejected by a CHECK constraint; sqlite3 output:\n{}",
        out_neg
    );
}

#[test]
#[serial]
fn agent_slot_records_survive_process_restart() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("agent_slot_restart");
    add_and_start(&h, "Agent Slot Restart");
    let db = db_path(&h);

    sqlite_query(
        &db,
        "INSERT INTO agent_slot \
         (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
         VALUES ('inst-persist', 0, 'claude', 'persisted-sess', '/tmp', 1);",
    );

    // "Close and reopen AoE" -> run the binary again as a fresh process.
    let reopen = h.run_cli(&["list"]);
    assert!(
        reopen.status.success(),
        "reopening aoe failed: {}",
        String::from_utf8_lossy(&reopen.stderr)
    );

    let value = sqlite_query(
        &db,
        "SELECT native_session_id FROM agent_slot \
         WHERE instance_id='inst-persist' AND slot=0;",
    );
    assert_eq!(
        value, "persisted-sess",
        "durable agent_slot row must survive a process restart"
    );
}

// ---------------------------------------------------------------------------
// Requirement: Append-only event stream
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn events_are_append_only_with_monotonic_ids() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("events_append_only");
    add_and_start(&h, "Events Append Only");
    let db = db_path(&h);

    sqlite_query(
        &db,
        "INSERT INTO events (instance_id, slot, kind, detail, created_at) \
         VALUES ('inst-e', 0, 'status', 'running', 1);",
    );
    let first_id = sqlite_query(
        &db,
        "SELECT id FROM events WHERE instance_id='inst-e' ORDER BY id DESC LIMIT 1;",
    );

    sqlite_query(
        &db,
        "INSERT INTO events (instance_id, slot, kind, detail, created_at) \
         VALUES ('inst-e', 0, 'capture', 'sess', 2);",
    );
    let second_id = sqlite_query(
        &db,
        "SELECT id FROM events WHERE instance_id='inst-e' ORDER BY id DESC LIMIT 1;",
    );

    let first: i64 = first_id.parse().expect("first event id is an integer");
    let second: i64 = second_id.parse().expect("second event id is an integer");
    assert!(
        second > first,
        "event id must be monotonically increasing (first={}, second={})",
        first,
        second
    );

    let count = sqlite_query(
        &db,
        "SELECT count(*) FROM events WHERE instance_id='inst-e';",
    );
    assert_eq!(
        count, "2",
        "existing event rows must not be modified or replaced"
    );
}

// ---------------------------------------------------------------------------
// Requirement: Store cleanup on session deletion
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn deleting_session_purges_durable_records() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("store_delete_cleanup");
    let instance_id = add_and_start(&h, "Store Delete Cleanup");
    let db = db_path(&h);

    // Seed durable + volatile rows tied to this instance/pane.
    sqlite_query(
        &db,
        &format!(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
             VALUES ('{instance_id}', 0, 'claude', 'sess-del', '/tmp', 1);"
        ),
    );
    sqlite_query(
        &db,
        "INSERT INTO pane_live (tmux_pane, agent, native_session_id, cwd, updated_at) \
         VALUES ('%9', 'claude', 'sess-del', '/tmp', 1);",
    );

    // Delete the session via the real CLI entry point.
    let remove = h.run_cli(&["remove", "Store Delete Cleanup", "--force"]);
    assert!(
        remove.status.success(),
        "aoe remove failed: {}",
        String::from_utf8_lossy(&remove.stderr)
    );

    let slot_count = sqlite_query(
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        slot_count, "0",
        "deleting a session must remove its agent_slot rows"
    );
}
