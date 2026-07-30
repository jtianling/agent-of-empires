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
    run_record_pane_as(h, tmux_pane, aoe_instance_id, stdin_json, None, &[])
}

/// The same, for an agent that is named and whose session id may come from its
/// environment rather than from the hook's stdin.
///
/// `env` entries with an empty value are removed rather than set, so a test can
/// state that a variable is absent instead of relying on the runner's
/// environment not to have it.
fn run_record_pane_as(
    h: &TuiTestHarness,
    tmux_pane: Option<&str>,
    aoe_instance_id: Option<&str>,
    stdin_json: &str,
    agent: Option<&str>,
    env: &[(&str, &str)],
) -> bool {
    let mut cmd = Command::new(h.binary_path());
    cmd.arg("__record-pane");
    if let Some(agent) = agent {
        cmd.args(["--agent", agent]);
    }
    for (key, value) in env {
        if value.is_empty() {
            cmd.env_remove(key);
        } else {
            cmd.env(key, value);
        }
    }
    // These tests simulate a hook from outside any pane, so keep the capture's
    // pane-ownership check unanswerable rather than answerably wrong: point it
    // at a serverless socket dir, and drop any real $TMUX so it cannot reach
    // the developer's own server. The dir MUST exist: tmux silently falls back
    // to the real default socket when $TMUX_TMPDIR does not.
    let no_server = h.home_path().join("no-tmux-server");
    std::fs::create_dir_all(&no_server).expect("create serverless tmpdir");
    cmd.env_remove("TMUX").env("TMUX_TMPDIR", &no_server);
    cmd.env("HOME", h.home_path())
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

/// A Codex capture reads the id off the hook's stdin, and `$CODEX_THREAD_ID`
/// does not enter into it. Codex exports that variable to the commands its
/// tools run but not to its hooks, so a capture that preferred it would record
/// nothing at all; the environment here carries a different value precisely so
/// that reading it would fail this test.
#[test]
#[serial]
fn codex_capture_reads_the_session_id_from_hook_stdin() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("capture_codex_thread_id");
    add_and_start(&h, "Capture Codex");
    let db = db_path(&h);

    let stdin_json = r#"{"session_id":"codex-thread-abc","cwd":"/work/codex"}"#;
    let ok = run_record_pane_as(
        &h,
        Some("%51"),
        Some("inst-codex"),
        stdin_json,
        Some("codex"),
        &[("CODEX_THREAD_ID", "stale-env-value")],
    );
    assert!(ok, "aoe __record-pane should exit 0 on a valid capture");

    let row = sqlite_query(
        &db,
        "SELECT agent || '|' || native_session_id || '|' || cwd FROM pane_live WHERE tmux_pane='%51';",
    );
    assert_eq!(
        row, "codex|codex-thread-abc|/work/codex",
        "a Codex capture takes both its id and its cwd from the hook's stdin"
    );
}

/// A hook event that carries no session id records nothing. A row with no
/// conversation on it is worse than no row: recovery would resume the pane
/// with no way to say which conversation it belongs to.
#[test]
#[serial]
fn a_capture_without_a_session_id_writes_no_row() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("capture_no_borrow");
    add_and_start(&h, "Capture No Borrow");
    let db = db_path(&h);

    let stdin_json = r#"{"cwd":"/work"}"#;
    let ok = run_record_pane_as(
        &h,
        Some("%52"),
        Some("inst-noborrow"),
        stdin_json,
        Some("codex"),
        &[("CODEX_THREAD_ID", "")],
    );
    assert!(ok, "a skipped capture must still exit 0");

    let count = sqlite_query(&db, "SELECT count(*) FROM pane_live WHERE tmux_pane='%52';");
    assert_eq!(
        count, "0",
        "a hook event with no session id on it must be skipped, not recorded \
         with an empty conversation"
    );
}

/// A `$TMUX_PANE` that checkably belongs to someone else is not recorded.
/// This is the shared-app-server failure measured live: every Codex session's
/// hooks inherited the daemon's own `$TMUX_PANE`, so each would have claimed
/// the daemon's pane -- an unrelated live session -- and recovery acts on
/// those rows.
#[test]
#[serial]
fn a_pane_that_belongs_to_another_process_is_not_claimed() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("capture_foreign_pane");
    let title = add_and_start(&h, "Capture Foreign Pane");
    let db = db_path(&h);

    // A real pane of the managed session, named from OUTSIDE it: the capture
    // can reach the harness server, resolve the pane's root process, and see
    // that this process is no descendant of it.
    let instance_id = instance_id_from_sessions_json(&h);
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, &title);
    let pane_id = h.tmux_display_message(&session_name, "#{pane_id}");

    let stdin_json = r#"{"session_id":"stolen-sess","cwd":"/work"}"#;
    let mut cmd = Command::new(h.binary_path());
    cmd.arg("__record-pane")
        .env_remove("TMUX")
        .env("TMUX_TMPDIR", h.tmux_tmpdir())
        .env("HOME", h.home_path())
        .env("XDG_CONFIG_HOME", h.home_path().join(".config"))
        .env("AGENT_OF_EMPIRES_PROFILE", "default")
        .env("TMUX_PANE", &pane_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn record-pane");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_json.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "a refused capture still exits 0");

    let count = sqlite_query(
        &db,
        &format!("SELECT count(*) FROM pane_live WHERE tmux_pane='{pane_id}';"),
    );
    assert_eq!(
        count, "0",
        "a pane hosted by another process must not be claimed"
    );
}

/// The check from the previous test must not eat legitimate captures: run the
/// same command from INSIDE the pane, the way a real hook fires, and the row
/// appears.
#[test]
#[serial]
fn a_capture_from_inside_its_own_pane_is_recorded() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let h = TuiTestHarness::new("capture_own_pane");
    let title = add_and_start(&h, "Capture Own Pane");
    let db = db_path(&h);

    let instance_id = instance_id_from_sessions_json(&h);
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, &title);
    let pane_id = h.tmux_display_message(&session_name, "#{pane_id}");

    // The managed pane runs `sh` (cmd-override); type the hook command into it
    // so the capture is a true descendant of the pane's own process.
    let json = r#"{\"session_id\":\"own-pane-sess\",\"cwd\":\"/work\"}"#;
    let capture_cmd = format!(
        "printf '%s' \"{json}\" | HOME={home} XDG_CONFIG_HOME={home}/.config \
         AGENT_OF_EMPIRES_PROFILE=default {bin} __record-pane",
        home = h.home_path().display(),
        bin = h.binary_path().display(),
    );
    h.send_keys_to_session(&session_name, &capture_cmd);

    wait_for_count(
        &h,
        &db,
        &format!(
            "SELECT count(*) FROM pane_live WHERE tmux_pane='{pane_id}' \
             AND native_session_id='own-pane-sess';"
        ),
        "1",
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

/// Id of the first (only) registered instance, read from sessions.json.
fn instance_id_from_sessions_json(h: &TuiTestHarness) -> String {
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
}

#[test]
#[serial]
fn reconciler_snapshots_pane_capture_into_slot() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("reconcile_snapshot");
    add_and_start(&h, "Reconcile Snapshot");
    let db = db_path(&h);

    let instance_id = instance_id_from_sessions_json(&h);

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

/// The behavior the whole Codex change exists for: a Codex pane reaching a
/// durable slot that records `codex`, bound to its conversation with no hook
/// involved -- the reconciler reads Codex's own rollout file, matching on the
/// instance's working directory and the pane's launch time.
#[test]
#[serial]
fn a_codex_pane_reaches_a_durable_slot_via_its_rollout_file() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("reconcile_codex_slot");
    // Loop instead of `exec sleep`: the claim requires a process whose argv
    // still names codex, the way the real npm shim's does.
    h.install_stub_script("codex", "#!/bin/sh\nwhile :; do sleep 60; done\n");

    let project = h.project_path();
    let add = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "Reconcile Codex",
        "-c",
        "codex",
    ]);
    assert!(
        add.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let start = h.run_cli_in_tmux(&["session", "start", "Reconcile Codex"]);
    assert!(
        start.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    let db = db_path(&h);

    let instance_id = instance_id_from_sessions_json(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Reconcile Codex");
    let pane_id = h.tmux_display_message(&session_name, "#{pane_id}");

    // The rollout file the running Codex would have written: created after the
    // pane started, in the instance's working directory.
    let thread_id = "0199aaaa-bbbb-cccc-dddd-eeeeffff0001";
    let now = chrono::Local::now();
    let day_dir = h
        .home_path()
        .join(".codex/sessions")
        .join(now.format("%Y/%m/%d").to_string());
    std::fs::create_dir_all(&day_dir).expect("create rollout dir");
    let meta = format!(
        "{{\"timestamp\":\"x\",\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"{thread_id}\",\"cwd\":\"{}\"}}}}\n",
        project.display()
    );
    std::fs::write(
        day_dir.join(format!(
            "rollout-{}-{thread_id}.jsonl",
            now.format("%Y-%m-%dT%H-%M-%S")
        )),
        meta,
    )
    .expect("write rollout file");

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    wait_for_count(
        &h,
        &db,
        &format!(
            "SELECT count(*) FROM pane_live WHERE tmux_pane='{pane_id}' \
             AND agent='codex' AND native_session_id='{thread_id}';"
        ),
        "1",
    );
    wait_for_count(
        &h,
        &db,
        &format!(
            "SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}' \
             AND agent='codex' AND native_session_id='{thread_id}';"
        ),
        "1",
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

    let instance_id = instance_id_from_sessions_json(&h);
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
