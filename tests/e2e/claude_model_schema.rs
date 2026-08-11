//! RED e2e tests for the `agent-session-store` delta: `agent_slot` gains a
//! `model` column, healed into legacy databases by the existing idempotent
//! schema path rather than by fallback logic in the main code.
//!
//! Shaped after `legacy_schema_heal`: `.schema_version` is pinned to the current
//! version so the migration system is a no-op, mirroring a real machine where
//! migrations already ran. The only thing left that can add the column is
//! `ensure_schema` on a normal store open, which is exactly what is under test.
//!
//! "Model survives process restart" is covered end-to-end in
//! `claude_model_continuity::model_survives_process_restart`, where the value
//! arrives through the real detection path rather than a hand-written row.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serial_test::serial;

use crate::claude_model_support::{
    assert_slot_model_stays, config_dir, db_path, instance_models, require_sqlite3,
    run_record_pane, seed_instance, sqlite_query, SlotSeed,
};
use crate::harness::TuiTestHarness;

/// Mirrors `src/migrations/mod.rs::CURRENT_VERSION`.
const CURRENT_SCHEMA_VERSION: &str = "10";

fn pin_schema_version(h: &TuiTestHarness) {
    std::fs::write(
        config_dir(h).join(".schema_version"),
        CURRENT_SCHEMA_VERSION,
    )
    .expect("write .schema_version");
}

/// Create an `agent_slot` table from before the `model` column, optionally with
/// one durable row that must survive the backfill.
fn create_legacy_agent_slot(db: &Path, seed_row: bool) {
    std::fs::create_dir_all(db.parent().expect("db parent")).expect("create profile dir");
    let mut sql = String::from(
        "CREATE TABLE agent_slot (\
            instance_id        TEXT NOT NULL,\
            slot               INTEGER NOT NULL CHECK (slot >= 0 AND slot <= 3),\
            agent              TEXT NOT NULL,\
            native_session_id  TEXT NOT NULL,\
            cwd                TEXT NOT NULL,\
            tmux_pane          TEXT NOT NULL DEFAULT '',\
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
        "creating the legacy agent_slot table failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn model_column_count(db: &Path) -> String {
    sqlite_query(
        db,
        "SELECT count(*) FROM pragma_table_info('agent_slot') WHERE name='model';",
    )
}

// ---------------------------------------------------------------------------
// Requirement: Schema healing covers the model column
// ---------------------------------------------------------------------------

/// Scenario: Legacy database gains the model column
#[test]
#[serial]
fn legacy_database_gains_the_model_column() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("model_schema_legacy");
    pin_schema_version(&h);
    let db = db_path(&h);
    create_legacy_agent_slot(&db, true);

    assert_eq!(
        model_column_count(&db),
        "0",
        "precondition: the legacy table must lack the model column"
    );

    // A real store open (`aoe __record-pane`) runs ensure_schema.
    assert!(
        run_record_pane(&h, "%1", "heal-inst", "claude", "heal-sess", "/work"),
        "aoe __record-pane should exit 0"
    );

    assert_eq!(
        model_column_count(&db),
        "1",
        "ensure_schema must backfill agent_slot.model on store open"
    );

    let row = sqlite_query(
        &db,
        "SELECT native_session_id || '|' || model FROM agent_slot \
         WHERE instance_id='legacy' AND slot=0;",
    );
    assert_eq!(
        row, "legacysess|",
        "the legacy row must survive the backfill, with model defaulting to ''"
    );

    // Writes must keep working against the healed table.
    assert!(
        run_record_pane(&h, "%2", "heal-inst", "claude", "heal-sess-2", "/work"),
        "a slot write after the backfill should succeed"
    );
}

/// Scenario: Model column healing is idempotent
#[test]
#[serial]
fn model_column_healing_is_idempotent() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("model_schema_idempotent");
    pin_schema_version(&h);
    let db = db_path(&h);
    create_legacy_agent_slot(&db, true);

    for pass in 1..=3 {
        assert!(
            run_record_pane(&h, "%1", "idem-inst", "claude", "idem-sess", "/work"),
            "store open pass {pass} should succeed"
        );
        assert_eq!(
            model_column_count(&db),
            "1",
            "pass {pass}: the schema path must never add a duplicate model column"
        );
    }

    let rows = sqlite_query(
        &db,
        "SELECT count(*) FROM agent_slot WHERE instance_id='legacy';",
    );
    assert_eq!(rows, "1", "repeated healing must leave existing rows alone");
}

// ---------------------------------------------------------------------------
// Requirement: Durable per-slot agent record (MODIFIED)
//   Scenario: 空 model capture 不清空已有值
// ---------------------------------------------------------------------------

/// A reconcile pass that observes nothing must not overwrite a stored model
/// with the empty string. Here the pane has no transcript at all, so every pass
/// carries an empty model while the row already holds one.
#[test]
#[serial]
fn empty_model_capture_does_not_clear_the_stored_value() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("model_schema_no_clobber");
    let native = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeee30";
    let fixture = seed_instance(
        &mut h,
        "Model No Clobber",
        "claude",
        None,
        &[SlotSeed {
            agent: "claude",
            native,
        }],
    );
    let db = db_path(&h);

    // No transcript exists for this pane, so nothing can be observed. Put a
    // value on the row directly and let the reconciler keep running over it.
    sqlite_query(
        &db,
        &format!(
            "UPDATE agent_slot SET model='claude-opus-5' \
             WHERE instance_id='{}' AND native_session_id='{native}';",
            fixture.instance_id
        ),
    );
    assert_eq!(
        instance_models(&db, &fixture.instance_id),
        vec!["claude-opus-5".to_string()],
        "precondition: the row must hold a model before the empty captures start"
    );

    assert_slot_model_stays(
        &db,
        &fixture.instance_id,
        native,
        "claude-opus-5",
        Duration::from_secs(6),
        "an empty capture must not clobber a stored model",
    );
}
