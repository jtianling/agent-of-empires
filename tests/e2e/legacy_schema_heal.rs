//! E2E acceptance for the `heal-agent-slot-schema` change.
//!
//! Real-machine `agent_slot` tables were created before the `tmux_pane` column
//! was added to the DDL. Because the table is created with
//! `CREATE TABLE IF NOT EXISTS`, those 6-column tables are never upgraded, and
//! `upsert_agent_slot` (which writes 7 columns including `tmux_pane`) fails with
//! "table agent_slot has no column named tmux_pane". The reconciler swallows the
//! error, so `agent_slot` stays permanently empty on those machines. Existing
//! e2e tests always start from a fresh db (which already has the column), so they
//! never exercise this path -- this file covers the legacy-db blind spot through
//! the real binary.
//!
//! The fix backfills the column inside `ensure_schema`, which runs on every
//! `Store::open_with_schema`. These tests pin `.schema_version` to the current
//! version so the migration system does NOT run (mirroring a real machine where
//! migrations are already done); the only thing that can heal the table is
//! `ensure_schema` on a normal store open.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serial_test::serial;

use crate::harness::TuiTestHarness;

/// Mirror the migration system's CURRENT_VERSION so pinning `.schema_version`
/// keeps the migrations from running in the test's isolated HOME.
const CURRENT_SCHEMA_VERSION: &str = "6";

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

/// The app/config directory inside the harness's isolated HOME.
fn config_dir(h: &TuiTestHarness) -> PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path().join(".config/agent-of-empires")
    } else {
        h.home_path().join(".agent-of-empires")
    }
}

fn db_path(h: &TuiTestHarness) -> PathBuf {
    config_dir(h).join("profiles/default/aoe.db")
}

/// Pin the schema version to current so `run_migrations` is a no-op. On a real
/// machine migrations have already run (version is current), so the only heal
/// path left is `ensure_schema` on store open -- which is what we want to test.
fn pin_schema_version(h: &TuiTestHarness) {
    std::fs::write(
        config_dir(h).join(".schema_version"),
        CURRENT_SCHEMA_VERSION,
    )
    .expect("write .schema_version");
}

/// Create a legacy `agent_slot` table (6 columns, no `tmux_pane`) at the profile
/// db path, mirroring a database created before the column was added. Optionally
/// seed one durable row to verify it survives the backfill.
fn create_legacy_agent_slot(db: &Path, seed_row: bool) {
    let mut sql = String::from(
        "CREATE TABLE agent_slot (\
            instance_id        TEXT NOT NULL,\
            slot               INTEGER NOT NULL CHECK (slot >= 0 AND slot <= 3),\
            agent              TEXT NOT NULL,\
            native_session_id  TEXT NOT NULL,\
            cwd                TEXT NOT NULL,\
            last_seen_at       INTEGER NOT NULL,\
            PRIMARY KEY (instance_id, slot)\
        );",
    );
    if seed_row {
        sql.push_str(
            "INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
             VALUES ('legacy', 0, 'claude', 'legacysess', '/tmp', 1);",
        );
    }
    let out = Command::new("sqlite3")
        .arg(db)
        .arg(&sql)
        .output()
        .expect("failed to create legacy db");
    assert!(
        out.status.success(),
        "creating legacy agent_slot failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn sqlite_query(db: &Path, sql: &str) -> String {
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

/// Invoke `aoe __record-pane` with a `$TMUX_PANE`, exactly as the hook would.
/// Opens the store with schema (record_pane.rs) -- the real heal trigger -- and
/// writes a `pane_live` row.
fn run_record_pane(
    h: &TuiTestHarness,
    tmux_pane: &str,
    instance_id: &str,
    session_id: &str,
) -> bool {
    let stdin_json = format!(
        "{{\"session_id\":\"{session_id}\",\"cwd\":\"/work\",\"hook_event_name\":\"SessionStart\"}}"
    );
    let mut child = Command::new(h.binary_path())
        .arg("__record-pane")
        .env("HOME", h.home_path())
        .env("XDG_CONFIG_HOME", h.home_path().join(".config"))
        .env("AGENT_OF_EMPIRES_PROFILE", "default")
        .env("TMUX_PANE", tmux_pane)
        .env("AOE_INSTANCE_ID", instance_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn aoe __record-pane");
    child
        .stdin
        .as_mut()
        .expect("record-pane stdin")
        .write_all(stdin_json.as_bytes())
        .expect("write record-pane stdin");
    child
        .wait_with_output()
        .expect("wait for aoe __record-pane")
        .status
        .success()
}

/// Register a session and start its tmux process. Returns the instance id.
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

    let sessions_path = config_dir(h).join("profiles/default/sessions.json");
    let content = std::fs::read_to_string(&sessions_path).expect("read sessions.json");
    let sessions: serde_json::Value = serde_json::from_str(&content).unwrap();
    sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str() == Some(title))
        .and_then(|s| s["id"].as_str())
        .unwrap_or_else(|| panic!("missing session {}", title))
        .to_string()
}

fn poll_count_eq(db: &Path, sql: &str, expected: &str, timeout: Duration) -> (bool, String) {
    let start = Instant::now();
    loop {
        let got = sqlite_query(db, sql);
        if got == expected {
            return (true, got);
        }
        if start.elapsed() > timeout {
            return (false, got);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ---------------------------------------------------------------------------
// Requirement: a legacy agent_slot table is healed on store open
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn store_open_backfills_legacy_agent_slot_schema() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("legacy_heal_backfill");
    pin_schema_version(&h);
    let db = db_path(&h);
    create_legacy_agent_slot(&db, true);

    // Precondition: the legacy table lacks tmux_pane.
    let before = sqlite_query(
        &db,
        "SELECT count(*) FROM pragma_table_info('agent_slot') WHERE name='tmux_pane';",
    );
    assert_eq!(
        before, "0",
        "precondition: legacy agent_slot must lack tmux_pane"
    );

    // A real store open (via __record-pane) runs ensure_schema, which backfills.
    assert!(
        run_record_pane(&h, "%1", "x", "s"),
        "aoe __record-pane should exit 0"
    );

    // The column is backfilled and the seeded legacy row is preserved (table
    // altered in place, not recreated) with tmux_pane defaulting to ''.
    let after = sqlite_query(
        &db,
        "SELECT count(*) FROM pragma_table_info('agent_slot') WHERE name='tmux_pane';",
    );
    assert_eq!(
        after, "1",
        "ensure_schema must backfill agent_slot.tmux_pane on open"
    );

    let row = sqlite_query(
        &db,
        "SELECT native_session_id || '|' || tmux_pane FROM agent_slot \
         WHERE instance_id='legacy' AND slot=0;",
    );
    assert_eq!(
        row, "legacysess|",
        "legacy row must survive the backfill; backfilled tmux_pane defaults to ''"
    );
}

// ---------------------------------------------------------------------------
// Requirement: the full capture + reconcile chain works on a legacy db
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn legacy_db_capture_reconcile_produces_agent_slot() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("legacy_heal_chain");
    pin_schema_version(&h);
    let db = db_path(&h);
    create_legacy_agent_slot(&db, false);

    let instance_id = add_and_start(&h, "Legacy Chain");
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, "Legacy Chain");
    let pane_id = h.tmux_display_message(&session_name, "#{pane_id}");

    assert!(
        run_record_pane(&h, &pane_id, &instance_id, "legacy-chain-sess"),
        "capture for the managed pane should succeed"
    );

    // Drive reconcile via the home-view status poller.
    h.spawn_tui();
    h.wait_for("Agent of Empires");

    // Before the fix this is impossible on a legacy db: upsert_agent_slot writes
    // tmux_pane, which errors on the 6-column table and is swallowed by the
    // reconciler, leaving agent_slot empty. With the backfill the column exists,
    // so reconcile upserts the captured row.
    let (ok, last) = poll_count_eq(
        &db,
        &format!(
            "SELECT count(*) FROM agent_slot \
             WHERE instance_id='{instance_id}' AND native_session_id='legacy-chain-sess';"
        ),
        "1",
        Duration::from_secs(12),
    );
    assert!(
        ok,
        "reconcile produced no agent_slot row on a legacy db (last count={last}); \
         the tmux_pane backfill did not heal the schema"
    );
}
