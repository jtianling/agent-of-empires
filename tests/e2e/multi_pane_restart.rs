//! E2E tests for the `multi-pane-resume-restart` capability (and the modified
//! `agent-resume-restart` behavior it supersedes).
//!
//! Pressing `R` on an instance must fan out the restart to EVERY tracked agent
//! pane recorded in `agent_slot` (up to 4), respawning each pane from its own
//! persisted `agent_slot.native_session_id`. For a pane whose agent supports
//! resume (claude, codex) the respawn command carries the resume flag built
//! from that id (`claude --resume <id>`, `codex resume <id>`). A pane with no
//! `ResumeConfig`, an empty `native_session_id`, or a failed resume degrades to
//! a fresh restart of that pane only, without blocking sibling panes.
//!
//! These tests drive the real `aoe` binary end-to-end (TUI via tmux). Slots are
//! populated through the real capture+reconcile path: `aoe __record-pane` writes
//! `pane_live` rows and the status-poller tick reconciles them into `agent_slot`.
//!
//! ## How a respawn is observed from outside the process
//!
//! The respawn is `tmux respawn-pane -k -c <cwd> -t <pane> <command>` (see
//! `src/tmux/session.rs::respawn_agent_pane`). tmux records the launched command
//! string in `#{pane_start_command}` for that pane, which survives even after the
//! (stubbed) agent binary exits. Each test therefore asserts on
//! `display-message -p '#{pane_start_command}'` per tracked pane id -- the
//! external, durable signal that the pane was respawned with `--resume <id>`.
//!
//! These tests pin the landed behavior: pressing `R` fans the restart out to
//! every tracked pane, so each sibling pane receives its own `--resume <id>`
//! start command rather than only the primary `@aoe_agent_pane`.

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

/// Invoke the hidden `aoe __record-pane` capture subcommand exactly as the hook
/// would: pipe hook stdin JSON, set `$TMUX_PANE`/`$AOE_INSTANCE_ID`, and pass an
/// optional `--agent` so non-default agents (codex, gemini) can be recorded.
fn run_record_pane(
    h: &TuiTestHarness,
    tmux_pane: &str,
    aoe_instance_id: &str,
    session_id: &str,
    agent: Option<&str>,
) -> bool {
    let stdin_json = format!(
        "{{\"session_id\":\"{session_id}\",\"cwd\":\"/work\",\"hook_event_name\":\"SessionStart\"}}"
    );
    let mut cmd = Command::new(h.binary_path());
    cmd.arg("__record-pane");
    if let Some(agent) = agent {
        cmd.arg("--agent").arg(agent);
    }
    let mut child = cmd
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

/// The command string tmux recorded for the pane at respawn time. After
/// `respawn-pane -k -t <pane> <command>` this reflects the resume command even
/// once the stubbed agent binary has exited.
fn pane_start_command(h: &TuiTestHarness, pane_id: &str) -> String {
    h.tmux_display_message(pane_id, "#{pane_start_command}")
}

/// Poll a pane's start command until it contains `needle`, or panic with the
/// last seen value and a screen dump. The `R` restart is asynchronous, so the
/// respawn command may take a tick to land.
fn wait_for_pane_start_command_contains(h: &TuiTestHarness, pane_id: &str, needle: &str) {
    let start = Instant::now();
    loop {
        let last = pane_start_command(h, pane_id);
        if last.contains(needle) {
            return;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!(
                "Timed out waiting for pane {} start command to contain {:?} (last={:?}).\
                 \n\n--- Screen ---\n{}",
                pane_id,
                needle,
                last,
                h.capture_screen()
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Establish `n` tracked agent panes (slots 0..n) for a started instance, each
/// captured + reconciled into `agent_slot`. Returns the tmux pane ids in slot
/// order (slot 0 is the primary `@aoe_agent_pane`). Each pane is recorded with
/// the agent at the matching index of `agents` (defaulting to `claude`).
fn establish_tracked_panes(
    h: &mut TuiTestHarness,
    instance_id: &str,
    session_name: &str,
    sessions: &[(&str, Option<&str>)],
    db: &std::path::Path,
) -> Vec<String> {
    h.resize_window(session_name, 220, 60);

    let primary = h.tmux_display_message(session_name, "#{pane_id}");
    let mut panes = vec![primary.clone()];
    let (sess0, agent0) = sessions[0];
    run_record_pane(h, &primary, instance_id, sess0, agent0);

    for &(sess, agent) in &sessions[1..] {
        let pane = h.split_window_get_pane(session_name);
        run_record_pane(h, &pane, instance_id, sess, agent);
        panes.push(pane);
    }

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    // Wait until the reconciler has recorded all panes as slots.
    wait_for_count(
        h,
        db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
        &sessions.len().to_string(),
    );

    panes
}

fn press_restart(h: &TuiTestHarness) {
    // `R` triggers Action::RespawnAgentPane for the selected (only) instance.
    h.send_keys("R");
}

// ---------------------------------------------------------------------------
// Requirement: R restart fans out to all tracked agent panes
// Requirement: Each tracked pane resumes from its persisted native session id
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn r_resumes_every_tracked_pane_from_its_own_id() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_resume_all");
    let instance_id = add_and_start(&h, "Multi Pane Resume All");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Multi Pane Resume All");

    // Three tracked claude panes, each with a distinct native_session_id.
    let panes = establish_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[
            ("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa0", None),
            ("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1", None),
            ("cccccccc-cccc-4ccc-8ccc-ccccccccccc2", None),
        ],
        &db,
    );

    press_restart(&h);

    // Every tracked pane must be respawned with its OWN persisted id, not just
    // the primary @aoe_agent_pane.
    wait_for_pane_start_command_contains(
        &h,
        &panes[0],
        "--resume aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa0",
    );
    wait_for_pane_start_command_contains(
        &h,
        &panes[1],
        "--resume bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1",
    );
    wait_for_pane_start_command_contains(
        &h,
        &panes[2],
        "--resume cccccccc-cccc-4ccc-8ccc-ccccccccccc2",
    );
}

#[test]
#[serial]
fn claude_pane_resume_command_has_resume_flag_and_no_exit_keys() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_claude_resume");
    let instance_id = add_and_start(&h, "Claude Resume Cmd");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Claude Resume Cmd");

    let panes = establish_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11", None)],
        &db,
    );

    press_restart(&h);

    // The respawn command must include `--resume <id>` after the claude binary.
    wait_for_pane_start_command_contains(
        &h,
        &panes[0],
        "claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
    );
}

#[test]
#[serial]
fn codex_pane_resume_command_uses_resume_subcommand() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_codex_resume");
    let instance_id = add_and_start(&h, "Codex Resume Cmd");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Codex Resume Cmd");

    let panes = establish_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[("019d1af9-a899-7df1-8f7d-a244126e5ded", Some("codex"))],
        &db,
    );

    press_restart(&h);

    // Codex uses the subcommand form: `codex resume <id>` (no leading `--`).
    wait_for_pane_start_command_contains(
        &h,
        &panes[0],
        "resume 019d1af9-a899-7df1-8f7d-a244126e5ded",
    );
}

// ---------------------------------------------------------------------------
// Requirement: R restart fans out to all tracked agent panes
//   Scenario: Single tracked pane behaves like the prior single-pane restart
//   Scenario: No tracked panes falls back to primary agent pane restart
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn single_tracked_pane_resumes_from_its_id() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_single_slot");
    let instance_id = add_and_start(&h, "Single Slot Resume");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Single Slot Resume");

    let panes = establish_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[("dddddddd-dddd-4ddd-8ddd-ddddddddddd0", None)],
        &db,
    );

    press_restart(&h);

    // Exactly one tracked pane (slot 0) -> that pane is resumed from its id.
    wait_for_pane_start_command_contains(
        &h,
        &panes[0],
        "--resume dddddddd-dddd-4ddd-8ddd-ddddddddddd0",
    );
}

#[test]
#[serial]
fn no_tracked_panes_restarts_primary_pane_fresh() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_no_slots");
    let instance_id = add_and_start(&h, "No Slots Fallback");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "No Slots Fallback");

    // No __record-pane capture -> reconciler records zero agent_slot rows.
    h.spawn_tui();
    h.wait_for("Agent of Empires");
    let slot_count = sqlite_query(
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        slot_count, "0",
        "precondition: instance must have no tracked agent_slot rows"
    );

    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    press_restart(&h);

    // Fallback: the primary @aoe_agent_pane is restarted fresh with the instance
    // launch command (here the `sh` stub from `--cmd-override`), with no resume
    // flag harvested from a nonexistent slot.
    wait_for_pane_start_command_contains(&h, &primary, "sh");
    let cmd = pane_start_command(&h, &primary);
    assert!(
        !cmd.contains("--resume"),
        "no-slots fallback must restart fresh (no --resume), got start command: {:?}",
        cmd
    );
}

// ---------------------------------------------------------------------------
// Requirement: Per-pane failure isolation
//   Scenario: Pane without resume support restarts fresh
//   Scenario: Pane with empty native session id restarts fresh
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn no_resume_pane_restarts_fresh_without_blocking_sibling() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_failure_isolation");
    let instance_id = add_and_start(&h, "Failure Isolation");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Failure Isolation");

    // Slot 0: gemini (no ResumeConfig) -> must restart fresh.
    // Slot 1: claude with a persisted id -> must resume.
    let panes = establish_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[
            ("gemini-sess-0", Some("gemini")),
            ("eeeeeeee-eeee-4eee-8eee-eeeeeeeeeee1", None),
        ],
        &db,
    );

    press_restart(&h);

    // The claude sibling resumes with its id...
    wait_for_pane_start_command_contains(
        &h,
        &panes[1],
        "--resume eeeeeeee-eeee-4eee-8eee-eeeeeeeeeee1",
    );

    // ...and the gemini pane restarts fresh (binary only, no resume flag), proving
    // its lack of resume support did not block the sibling and did not error.
    wait_for_pane_start_command_contains(&h, &panes[0], "gemini");
    let gemini_cmd = pane_start_command(&h, &panes[0]);
    assert!(
        !gemini_cmd.contains("resume"),
        "a no-ResumeConfig pane must restart fresh (no resume flag), got: {:?}",
        gemini_cmd
    );
}

#[test]
#[serial]
fn empty_native_session_id_restarts_pane_fresh() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_empty_id");
    let instance_id = add_and_start(&h, "Empty Id Fresh");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Empty Id Fresh");

    // Establish one real tracked claude pane so a slot row exists, then null out
    // its native_session_id to model a slot with no usable resume id.
    let panes = establish_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[("to-be-cleared", None)],
        &db,
    );
    sqlite_query(
        &db,
        &format!(
            "UPDATE agent_slot SET native_session_id='' \
             WHERE instance_id='{instance_id}' AND slot=0;"
        ),
    );

    press_restart(&h);

    // The claude pane has a ResumeConfig but an empty id -> respawn fresh.
    wait_for_pane_start_command_contains(&h, &panes[0], "claude");
    let cmd = pane_start_command(&h, &panes[0]);
    assert!(
        !cmd.contains("--resume"),
        "a claude pane with an empty native_session_id must restart fresh, got: {:?}",
        cmd
    );
}

// ---------------------------------------------------------------------------
// DEFERRED scenarios (not generated -- see the module summary in the create-test
// run output):
//   - multi-pane-resume-restart / "Process tree killed before respawn":
//     asserts an internal kill-only-this-pane invariant on the process tree;
//     there is no externally-observable signal distinguishing per-pane kill from
//     the visible respawn (the respawn command, which IS observable, is already
//     covered above). No real user-entry observation point.
//   - multi-pane-resume-restart / tmux-pane-operations scenarios
//     ("Respawn targets the specified pane %37", "Process kill targets %37"):
//     these pin the exact tmux argv (`respawn-pane -k -t %37`) of an internal
//     helper -- unit-test territory (tasks 1.3), not an e2e user entry.
//   - multi-pane-resume-restart / "Failed resume respawn does not abort sibling
//     panes": requires forcing one pane's tmux respawn to error from outside,
//     for which the binary exposes no fault-injection entry point.
//   - multi-pane-resume-restart / "Status reflects in-flight multi-pane restart"
//     and "Duplicate R press during multi-pane restart is ignored": the stubbed
//     agent binary exits instantly, so the transient `Restarting` window is not
//     deterministically observable via screen scrape; needs a controllable
//     long-lived agent the harness does not provide.
// ---------------------------------------------------------------------------
