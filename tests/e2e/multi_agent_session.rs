//! RED e2e tests for the `multi-agent-session` capability.
//!
//! A managed session tracks up to four agent slots (0..3). Agents appearing in
//! any pane of a managed session are adopted (observe-first) and recorded in
//! `agent_slot`, with an `adopt` event appended. The four-slot cap is enforced.
//!
//! All tests are RED until the feature lands: adoption, the `agent_slot`/`events`
//! tables, and the reconciler do not exist yet.
//!
//! NOTE: The two "Optional add-agent-pane action" scenarios are DEFERRED, not
//! generated. design.md (Open Questions) leaves the exact trigger undecided
//! ("TUI key vs CLI subcommand ... to be finalized during apply"), so there is
//! no determinable real user entry point to drive an e2e test against yet.
//!
//! Tests drive the real `aoe` binary end-to-end (subprocess + tmux) and observe
//! `aoe.db` from outside via the `sqlite3` CLI.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

fn run_record_pane(
    h: &TuiTestHarness,
    tmux_pane: &str,
    aoe_instance_id: &str,
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
        .env("AOE_INSTANCE_ID", aoe_instance_id)
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

    let sessions_path = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    };
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

fn split_pane(h: &TuiTestHarness, session_name: &str) {
    let out = Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args(["split-window", "-t", session_name, "-d"])
        .output()
        .expect("split-window");
    assert!(
        out.status.success(),
        "split-window failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn wait_for_count(h: &TuiTestHarness, db: &std::path::Path, sql: &str, expected: &str) {
    let start = Instant::now();
    loop {
        let got = sqlite_query(db, sql);
        if got == expected {
            return;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!(
                "Timed out waiting for `{}` to equal {} (last={}).\n\n--- Screen ---\n{}",
                sql,
                expected,
                got,
                h.capture_screen()
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

// ---------------------------------------------------------------------------
// Requirement: A session tracks up to four agent slots
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn primary_pane_occupies_a_slot() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("primary_pane_slot");
    let instance_id = add_and_start(&h, "Primary Pane Slot");
    let db = db_path(&h);

    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Primary Pane Slot");
    let pane_id = h.tmux_display_message(&session_name, "#{pane_id}");
    assert!(
        run_record_pane(&h, &pane_id, &instance_id, "primary-sess"),
        "capture for the primary pane should succeed"
    );

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    // The primary managed agent must be tracked as one of the session's slots.
    wait_for_count(
        &h,
        &db,
        &format!(
            "SELECT count(*) FROM agent_slot \
             WHERE instance_id='{instance_id}' AND native_session_id='primary-sess';"
        ),
        "1",
    );
}

#[test]
#[serial]
fn tracking_caps_at_four() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("tracking_cap_four");
    let instance_id = add_and_start(&h, "Tracking Cap Four");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Tracking Cap Four");

    // First establish four REAL tracked panes (primary + three splits), each
    // captured, so the reconciler records slots 0..3 mapped to those panes.
    h.resize_window(&session_name, 220, 60);
    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    run_record_pane(&h, &primary, &instance_id, "real-0");
    for i in 1..4 {
        let pane = h.split_window_get_pane(&session_name);
        run_record_pane(&h, &pane, &instance_id, &format!("real-{i}"));
    }

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    // Wait until the four panes are tracked (sticky slots 0..3 now established).
    wait_for_count(
        &h,
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
        "4",
    );

    // Now a fifth agent pane appears with a capture. The cap must drop it, and
    // the four already-tracked slots must remain unchanged (sticky assignment).
    let fifth_pane = h.split_window_get_pane(&session_name);
    run_record_pane(&h, &fifth_pane, &instance_id, "fifth-sess");
    std::thread::sleep(Duration::from_secs(3));

    let count = sqlite_query(
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        count, "4",
        "a session already tracking four panes must not create a fifth slot"
    );

    // The four originally-tracked captures remain; the fifth is not recorded.
    let reals = sqlite_query(
        &db,
        &format!(
            "SELECT count(*) FROM agent_slot \
             WHERE instance_id='{instance_id}' AND native_session_id LIKE 'real-%';"
        ),
    );
    assert_eq!(reals, "4", "existing four slots must remain unchanged");
    let fifth = sqlite_query(
        &db,
        &format!(
            "SELECT count(*) FROM agent_slot \
             WHERE instance_id='{instance_id}' AND native_session_id='fifth-sess';"
        ),
    );
    assert_eq!(fifth, "0", "the fifth agent must not be recorded");
}

// ---------------------------------------------------------------------------
// Requirement: Agents appearing in any pane are adopted
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn agent_in_user_split_pane_is_adopted() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("adopt_split_pane");
    let instance_id = add_and_start(&h, "Adopt Split Pane");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Adopt Split Pane");

    // User creates a split pane and runs an agent there (capture appears).
    split_pane(&h, &session_name);
    let split_pane_id = h.tmux_display_message(&format!("{session_name}.1"), "#{pane_id}");
    run_record_pane(&h, &split_pane_id, &instance_id, "adopted-sess");

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    // The system must assign that pane a slot and record it in agent_slot.
    wait_for_count(
        &h,
        &db,
        &format!(
            "SELECT count(*) FROM agent_slot \
             WHERE instance_id='{instance_id}' AND native_session_id='adopted-sess';"
        ),
        "1",
    );
}

#[test]
#[serial]
fn adoption_is_recorded_as_an_event() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("adopt_event");
    let instance_id = add_and_start(&h, "Adopt Event");
    let db = db_path(&h);
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, "Adopt Event");

    split_pane(&h, &session_name);
    let split_pane_id = h.tmux_display_message(&format!("{session_name}.1"), "#{pane_id}");
    run_record_pane(&h, &split_pane_id, &instance_id, "adopt-evt-sess");

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    // Adopting a previously untracked pane must append an `adopt` event.
    wait_for_count(
        &h,
        &db,
        &format!(
            "SELECT count(*) FROM events \
             WHERE instance_id='{instance_id}' AND kind='adopt';"
        ),
        "1",
    );
}

// ---------------------------------------------------------------------------
// DEFERRED: Optional add-agent-pane action
//
// Two scenarios ("Add-agent-pane creates and tracks a new pane",
// "Add-agent-pane blocked at the cap") are NOT generated. The user-facing
// trigger for this action (TUI keybinding vs CLI subcommand) is explicitly
// left undecided in design.md Open Questions, so there is no determinable real
// entry point to drive an e2e test against yet. Revisit once the trigger is
// finalized during apply.
// ---------------------------------------------------------------------------
