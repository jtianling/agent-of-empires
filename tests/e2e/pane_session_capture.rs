//! RED e2e tests for the `pane-session-capture` capability.
//!
//! The capture path is the hidden `aoe __record-pane` subcommand that the
//! installed status hook shells out to: it reads hook stdin JSON (`.session_id`,
//! `.cwd`), reads `$TMUX_PANE` from the environment, and upserts a `pane_live`
//! row. The reconciler (driven on the status-poller tick) snapshots `pane_live`
//! captures into durable `agent_slot` rows and garbage-collects orphans.
//!
//! All tests are RED until the feature lands: the `__record-pane` subcommand,
//! the `pane_live`/`agent_slot` tables, and the reconciler do not exist yet.
//!
//! Tests drive the real `aoe` binary end-to-end (subprocess + tmux) and observe
//! `aoe.db` from outside via the `sqlite3` CLI.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serial_test::serial;

use crate::harness::TuiTestHarness;

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

fn db_path(h: &TuiTestHarness) -> PathBuf {
    let profile_dir = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default")
    } else {
        h.home_path().join(".agent-of-empires/profiles/default")
    };
    profile_dir.join("aoe.db")
}

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

/// Invoke the hidden `aoe __record-pane` capture subcommand exactly as the hook
/// would: pipe hook stdin JSON, set `$TMUX_PANE` (and optionally `$AOE_INSTANCE_ID`),
/// and run the real binary with the harness's env isolation. Returns the exit
/// status success flag.
fn run_record_pane(
    h: &TuiTestHarness,
    tmux_pane: Option<&str>,
    aoe_instance_id: Option<&str>,
    stdin_json: &str,
) -> bool {
    let mut cmd = Command::new(h.binary_path());
    cmd.arg("__record-pane")
        .env("HOME", h.home_path())
        .env("XDG_CONFIG_HOME", h.home_path().join(".config"))
        .env("AGENT_OF_EMPIRES_PROFILE", "default")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match tmux_pane {
        Some(pane) => {
            cmd.env("TMUX_PANE", pane);
        }
        None => {
            cmd.env_remove("TMUX_PANE");
        }
    }
    match aoe_instance_id {
        Some(id) => {
            cmd.env("AOE_INSTANCE_ID", id);
        }
        None => {
            cmd.env_remove("AOE_INSTANCE_ID");
        }
    }

    let mut child = cmd.spawn().expect("failed to spawn aoe __record-pane");
    child
        .stdin
        .as_mut()
        .expect("record-pane stdin")
        .write_all(stdin_json.as_bytes())
        .expect("write record-pane stdin");
    let output = child
        .wait_with_output()
        .expect("wait for aoe __record-pane");
    output.status.success()
}

/// Register a session and start its tmux process to initialize the store and
/// produce a managed session with a real pane.
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

    title.to_string()
}

// ---------------------------------------------------------------------------
// Requirement: Hook captures native session id keyed by tmux pane
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn capture_reads_session_id_from_stdin() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("capture_stdin_session_id");
    add_and_start(&h, "Capture Stdin");
    let db = db_path(&h);

    let stdin_json =
        r#"{"session_id":"claude-sess-123","cwd":"/work/dir","hook_event_name":"SessionStart"}"#;
    let ok = run_record_pane(&h, Some("%42"), Some("inst-cap"), stdin_json);
    assert!(ok, "aoe __record-pane should exit 0 on a valid capture");

    let row = sqlite_query(
        &db,
        "SELECT native_session_id || '|' || cwd FROM pane_live WHERE tmux_pane='%42';",
    );
    assert_eq!(
        row, "claude-sess-123|/work/dir",
        "pane_live row must carry the stdin session_id and cwd keyed by $TMUX_PANE"
    );
}

#[test]
#[serial]
fn capture_works_without_aoe_instance_id() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("capture_hand_launched");
    add_and_start(&h, "Capture Hand Launched");
    let db = db_path(&h);

    // Hand-launched agent: no $AOE_INSTANCE_ID, but a real $TMUX_PANE is present.
    let stdin_json =
        r#"{"session_id":"hand-sess-9","cwd":"/home/me","hook_event_name":"SessionStart"}"#;
    let ok = run_record_pane(&h, Some("%77"), None, stdin_json);
    assert!(
        ok,
        "capture must not depend on $AOE_INSTANCE_ID; subcommand should exit 0"
    );

    let value = sqlite_query(
        &db,
        "SELECT native_session_id FROM pane_live WHERE tmux_pane='%77';",
    );
    assert_eq!(
        value, "hand-sess-9",
        "hand-launched agent (no $AOE_INSTANCE_ID) must still be captured"
    );
}

#[test]
#[serial]
fn capture_no_ops_outside_tmux() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("capture_outside_tmux");
    add_and_start(&h, "Capture Outside Tmux");
    let db = db_path(&h);

    let before = sqlite_query(&db, "SELECT count(*) FROM pane_live;");

    // No $TMUX_PANE -> the hook must not write a capture row, and must exit 0.
    let stdin_json =
        r#"{"session_id":"no-tmux-sess","cwd":"/tmp","hook_event_name":"SessionStart"}"#;
    let ok = run_record_pane(&h, None, Some("inst-x"), stdin_json);
    assert!(ok, "capture must exit 0 even when not inside tmux");

    let after = sqlite_query(&db, "SELECT count(*) FROM pane_live;");
    assert_eq!(
        before, after,
        "no pane_live row may be written when $TMUX_PANE is empty"
    );
}

// ---------------------------------------------------------------------------
// Requirement: Reconciler snapshots pane captures into durable slots
// ---------------------------------------------------------------------------

/// Poll until the given SQL count query reaches the expected value, or panic
/// with a screen dump. Used to wait for the reconciler tick to fire.
fn wait_for_count(h: &TuiTestHarness, db: &std::path::Path, sql: &str, expected: &str) {
    let start = std::time::Instant::now();
    loop {
        let got = sqlite_query(db, sql);
        if got == expected {
            return;
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            panic!(
                "Timed out waiting for `{}` to equal {} (last={}).\n\n--- Screen ---\n{}",
                sql,
                expected,
                got,
                h.capture_screen()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

#[test]
#[serial]
fn reconciler_snapshots_pane_capture_into_slot() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("reconcile_snapshot");
    add_and_start(&h, "Reconcile Snapshot");
    let db = db_path(&h);

    let instance_id = {
        let sessions_path = if cfg!(target_os = "linux") {
            h.home_path()
                .join(".config/agent-of-empires/profiles/default/sessions.json")
        } else {
            h.home_path()
                .join(".agent-of-empires/profiles/default/sessions.json")
        };
        let content = std::fs::read_to_string(&sessions_path).expect("read sessions.json");
        let sessions: serde_json::Value = serde_json::from_str(&content).unwrap();
        sessions.as_array().unwrap()[0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // Resolve the managed session's primary pane id.
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Reconcile Snapshot");
    let pane_id = h.tmux_display_message(&session_name, "#{pane_id}");

    // Simulate a hook capture landing for that pane.
    let stdin_json =
        r#"{"session_id":"reconcile-sess","cwd":"/work","hook_event_name":"SessionStart"}"#;
    let ok = run_record_pane(&h, Some(&pane_id), Some(&instance_id), stdin_json);
    assert!(ok, "capture should succeed for the managed pane");

    // Drive the TUI so the status-poller tick runs the reconciler.
    h.spawn_tui();
    h.wait_for("Agent of Empires");

    wait_for_count(
        &h,
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
        "1",
    );

    let value = sqlite_query(
        &db,
        &format!("SELECT native_session_id FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        value, "reconcile-sess",
        "reconciler must snapshot the pane capture into an agent_slot row"
    );
}

#[test]
#[serial]
fn reconciler_caps_at_four_slots() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("reconcile_four_cap");
    add_and_start(&h, "Reconcile Four Cap");
    let db = db_path(&h);

    let sessions_path = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    };
    let content = std::fs::read_to_string(&sessions_path).expect("read sessions.json");
    let sessions: serde_json::Value = serde_json::from_str(&content).unwrap();
    let instance_id = sessions.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Reconcile Four Cap");

    // Enlarge the detached session window so six panes fit (default 80x24 fails
    // multi-pane splits with "no space for new pane").
    h.resize_window(&session_name, 220, 60);

    // Create five extra panes (six total) each with a capture.
    for i in 0..5 {
        let pane_id = h.split_window_get_pane(&session_name);
        let stdin_json = format!(
            "{{\"session_id\":\"sess-{i}\",\"cwd\":\"/work\",\"hook_event_name\":\"SessionStart\"}}"
        );
        run_record_pane(&h, Some(&pane_id), Some(&instance_id), &stdin_json);
    }

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    // Let the reconciler tick run, then assert the cap.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let count = sqlite_query(
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    let n: i64 = count.parse().unwrap_or(99);
    assert!(
        n <= 4,
        "reconciler must record at most four agent_slot rows per session, got {}",
        n
    );
}

#[test]
#[serial]
fn reconciler_garbage_collects_orphan_captures() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("reconcile_orphan_gc");
    add_and_start(&h, "Reconcile Orphan GC");
    let db = db_path(&h);

    // A pane_live row whose tmux_pane belongs to no managed session.
    sqlite_query(
        &db,
        "INSERT INTO pane_live (tmux_pane, agent, native_session_id, cwd, updated_at) \
         VALUES ('%9999', 'claude', 'orphan-sess', '/tmp', 1);",
    );

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    wait_for_count(
        &h,
        &db,
        "SELECT count(*) FROM pane_live WHERE tmux_pane='%9999';",
        "0",
    );
}
