//! E2E tests for the `background-restart` capability (OpenSpec change
//! `async-home-restart`).
//!
//! Lowercase `c` (fresh) and `r` (resume) on the home view must enqueue the
//! restart onto a background serial worker and return control to the event
//! loop immediately. While the restart is in flight the instance is marked
//! `Restarting` / `restart_in_flight` and the home view rejects attach, delete
//! and further restart keys for it; the worker's result then merges back
//! (identity fields, per-pane errors, status `Starting`) and re-enables them.
//! Uppercase `C`/`R` keep their synchronous attach behavior.
//!
//! ## How "the restart takes a while" is constructed
//!
//! Every test enables Cross Agent Team on a Claude instance whose `claude`
//! binary is a sleep-forever stub. The restart pipeline then runs
//! `auto_confirm_panes`, which polls the pane screen for Claude's startup
//! prompt until its 12s deadline -- the stub never renders one, so the
//! pipeline reliably occupies the restart path for ~12 seconds. Today that
//! entire window runs synchronously inside the TUI event loop.
//!
//! ## How the background property is observed from outside the process
//!
//! - The status bar renders `Restarting...` for the selected instance only
//!   while the event loop is redrawing. A synchronous restart never redraws
//!   mid-pipeline, so today the text never appears -- these tests are RED
//!   until the restart actually runs in the background.
//! - Responsiveness is probed with a real keystroke (`o` cycles the sort
//!   order, flipping the visible `Sort: Newest` label to `Sort: Oldest`).
//! - The respawn itself is observed via `#{pane_start_command}` per pane
//!   (same durable signal `multi_pane_restart.rs` uses), and merged results
//!   via `sessions.json` (`agent_session_id`, `resume_token`, `status`) and
//!   the preview panel's `Error:` rendering of `last_error`.
//!
//! All tmux traffic goes through `TuiTestHarness` and its private per-test
//! socket; nothing here can reach the developer's real tmux server.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serial_test::serial;

use crate::harness::TuiTestHarness;

/// The status-bar marker rendered for a selected instance in `Restarting`.
const RESTARTING: &str = "Restarting...";

/// The Cross Agent Team flag a relaunched Claude pane carries. The initial
/// launch in these tests happens BEFORE Cross Agent Team is enabled, so this
/// flag appearing in `#{pane_start_command}` proves the pane was respawned by
/// the restart under test rather than still running its original command.
const CAT_FLAG: &str = "--dangerously-load-development-channels";

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

fn profile_dir(h: &TuiTestHarness) -> PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default")
    } else {
        h.home_path().join(".agent-of-empires/profiles/default")
    }
}

fn db_path(h: &TuiTestHarness) -> PathBuf {
    profile_dir(h).join("aoe.db")
}

fn sessions_json_path(h: &TuiTestHarness) -> PathBuf {
    profile_dir(h).join("sessions.json")
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

/// Run a tmux subcommand against the harness's private socket. `-S` with an
/// absolute path is authoritative over any inherited `$TMUX`, and the env is
/// scrubbed anyway so this can never reach the developer's own server.
fn tmux(h: &TuiTestHarness, args: &[&str]) -> Output {
    Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args(args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("failed to run tmux")
}

fn session_exists(h: &TuiTestHarness, session: &str) -> bool {
    tmux(h, &["has-session", "-t", session]).status.success()
}

fn read_sessions(h: &TuiTestHarness) -> serde_json::Value {
    let content = std::fs::read_to_string(sessions_json_path(h)).expect("read sessions.json");
    serde_json::from_str(&content).expect("parse sessions.json")
}

fn write_sessions(h: &TuiTestHarness, sessions: &serde_json::Value) {
    std::fs::write(
        sessions_json_path(h),
        serde_json::to_string_pretty(sessions).expect("serialize sessions.json"),
    )
    .expect("write sessions.json");
}

fn session_field(h: &TuiTestHarness, title: &str, field: &str) -> Option<String> {
    let sessions = read_sessions(h);
    sessions
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|s| s["title"].as_str() == Some(title))
        .and_then(|s| s[field].as_str())
        .map(str::to_string)
}

fn instance_id_for(h: &TuiTestHarness, title: &str) -> String {
    session_field(h, title, "id")
        .unwrap_or_else(|| panic!("missing session {title} in sessions.json"))
}

/// Install a long-lived `claude` stub (so the launched pane survives to be
/// respawned), add an instance for it and start its tmux session. Returns the
/// instance id.
fn add_and_start(h: &TuiTestHarness, title: &str) -> String {
    h.install_tool_stub("claude");
    let project = h.project_path();
    let add = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        title,
        "-c",
        "claude",
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
    instance_id_for(h, title)
}

/// Turn Cross Agent Team on for an already-created session, the way
/// `cold_start_recovery.rs` does: there is no CLI flag, so it is written into
/// the store before the TUI loads it. The pane owns the flag (the instance
/// field is derived), and the launch that already happened provisioned slot 0
/// with the flag off, so both the JSON record and the slot row must change.
fn enable_cross_agent_team(h: &TuiTestHarness, title: &str) {
    let mut sessions = read_sessions(h);
    let session = sessions
        .as_array_mut()
        .expect("sessions array")
        .iter_mut()
        .find(|s| s["title"] == title)
        .expect("created session");
    session["cross_agent_team"] = serde_json::Value::Bool(true);
    session["primary_pane"]["cross_agent_team"] = serde_json::Value::Bool(true);
    let instance_id = session["id"].as_str().expect("session id").to_string();
    write_sessions(h, &sessions);

    sqlite_query(
        &db_path(h),
        &format!("UPDATE agent_slot SET cross_agent_team=1 WHERE instance_id='{instance_id}';"),
    );
}

/// Re-assert (and if necessary re-apply) the Cross Agent Team flag on every
/// slot row right before a restart keypress, so the 12-second auto-confirm
/// window the tests rely on is a checked precondition rather than a hope.
fn force_cat_on_slots(h: &TuiTestHarness, instance_id: &str) {
    let db = db_path(h);
    sqlite_query(
        &db,
        &format!("UPDATE agent_slot SET cross_agent_team=1 WHERE instance_id='{instance_id}';"),
    );
    let total = sqlite_query(
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    let flagged = sqlite_query(
        &db,
        &format!(
            "SELECT count(*) FROM agent_slot \
             WHERE instance_id='{instance_id}' AND cross_agent_team=1;"
        ),
    );
    assert!(
        total != "0" && flagged == total,
        "precondition: every tracked slot must carry Cross Agent Team \
         (total={total}, flagged={flagged}); without it the restart pipeline \
         skips auto-confirm and the in-flight window collapses"
    );
}

/// Invoke the hidden `aoe __record-pane` capture subcommand exactly as the
/// SessionStart hook would, so the reconciler fills the pane's slot with a
/// real `native_session_id`. `cwd` must be a real on-disk directory.
fn run_record_pane(
    h: &TuiTestHarness,
    tmux_pane: &str,
    aoe_instance_id: &str,
    session_id: &str,
    cwd: &str,
) {
    let stdin_json = format!(
        "{{\"session_id\":\"{session_id}\",\"cwd\":\"{cwd}\",\"hook_event_name\":\"SessionStart\"}}"
    );
    // Simulate a hook from outside the pane: point the capture's
    // pane-ownership check at a serverless socket dir (which MUST exist --
    // tmux silently falls back to the real default socket when $TMUX_TMPDIR
    // does not) and drop any real $TMUX.
    let no_server = h.home_path().join("no-tmux-server");
    std::fs::create_dir_all(&no_server).expect("create serverless tmpdir");
    let mut child = Command::new(h.binary_path())
        .arg("__record-pane")
        .arg("--agent")
        .arg("claude")
        .env_remove("TMUX")
        .env("TMUX_TMPDIR", &no_server)
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
    let out = child
        .wait_with_output()
        .expect("wait for aoe __record-pane");
    assert!(
        out.status.success(),
        "aoe __record-pane failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn wait_for_sql(h: &TuiTestHarness, sql: &str, expected: &str, timeout: Duration) {
    let db = db_path(h);
    let start = Instant::now();
    loop {
        let got = sqlite_query(&db, sql);
        if got == expected {
            return;
        }
        if start.elapsed() > timeout {
            panic!(
                "Timed out waiting for `{}` to equal {:?} (last={:?}).\n\n--- Screen ---\n{}",
                sql,
                expected,
                got,
                h.capture_screen()
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn pane_start_command(h: &TuiTestHarness, pane_id: &str) -> String {
    h.tmux_display_message(pane_id, "#{pane_start_command}")
}

fn wait_for_pane_start_command_contains(
    h: &TuiTestHarness,
    pane_id: &str,
    needle: &str,
    timeout: Duration,
) {
    let start = Instant::now();
    loop {
        let last = pane_start_command(h, pane_id);
        if last.contains(needle) {
            return;
        }
        if start.elapsed() > timeout {
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

/// Assert `text` stays on screen for the whole `hold` window.
fn assert_stays_on_screen(h: &TuiTestHarness, text: &str, hold: Duration) {
    let start = Instant::now();
    while start.elapsed() < hold {
        let screen = h.capture_screen();
        assert!(
            screen.contains(text),
            "Expected {:?} to stay on screen for {:?}, gone after {:?}.\
             \n\n--- Screen ---\n{}",
            text,
            hold,
            start.elapsed(),
            screen
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Assert `text` never appears on screen during the whole `hold` window.
fn assert_stays_absent(h: &TuiTestHarness, text: &str, hold: Duration) {
    let start = Instant::now();
    while start.elapsed() < hold {
        let screen = h.capture_screen();
        assert!(
            !screen.contains(text),
            "Expected {:?} to stay absent for {:?}, appeared after {:?}.\
             \n\n--- Screen ---\n{}",
            text,
            hold,
            start.elapsed(),
            screen
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Poll `sessions.json` until the named session's `field` satisfies `pred`.
/// Returns the value that satisfied it.
fn wait_for_session_field(
    h: &TuiTestHarness,
    title: &str,
    field: &str,
    pred: impl Fn(Option<&str>) -> bool,
    timeout: Duration,
) -> Option<String> {
    let start = Instant::now();
    loop {
        let value = session_field(h, title, field);
        if pred(value.as_deref()) {
            return value;
        }
        if start.elapsed() > timeout {
            panic!(
                "Timed out waiting for sessions.json field {:?} of {:?} (last={:?}).\
                 \n\n--- Screen ---\n{}",
                field,
                title,
                value,
                h.capture_screen()
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// One Cross Agent Team Claude instance, started and visible in the home TUI.
/// Returns `(instance_id, managed_session_name, primary_pane_id)`.
fn setup_cat_instance(h: &mut TuiTestHarness, title: &str) -> (String, String, String) {
    setup_cat_instance_impl(h, title, false)
}

/// Like [`setup_cat_instance`], but the TUI process runs with `$TMUX` scrubbed
/// (see `TuiTestHarness::spawn_tui_without_tmux_env`). Required by any test
/// that must prove a REAL attach: tmux refuses `attach-session` from a client
/// whose `$TMUX` is set and whose tty is one of the server's own panes, so
/// from an un-scrubbed TUI the attach can never succeed in this topology.
fn setup_cat_instance_outside_tmux_env(
    h: &mut TuiTestHarness,
    title: &str,
) -> (String, String, String) {
    setup_cat_instance_impl(h, title, true)
}

fn setup_cat_instance_impl(
    h: &mut TuiTestHarness,
    title: &str,
    scrub_tmux_env: bool,
) -> (String, String, String) {
    let instance_id = add_and_start(h, title);
    enable_cross_agent_team(h, title);
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    if scrub_tmux_env {
        h.spawn_tui_without_tmux_env();
    } else {
        h.spawn_tui();
    }
    h.wait_for("Agent of Empires");
    h.wait_for(title);
    force_cat_on_slots(h, &instance_id);
    (instance_id, session_name, primary)
}

/// Poll the private server until a client is attached to `session`. A really
/// attached client is the durable proof of a successful `attach-session`; a
/// refused or bounced attach never leaves one behind.
fn wait_for_client_on_session(h: &TuiTestHarness, session: &str, timeout: Duration) {
    let start = Instant::now();
    loop {
        let out = tmux(h, &["list-clients", "-F", "#{session_name}"]);
        if String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|line| line.trim() == session)
        {
            return;
        }
        if start.elapsed() > timeout {
            panic!(
                "Timed out waiting for an attached client on {:?}.\n\n--- Screen ---\n{}",
                session,
                h.capture_screen()
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

// ---------------------------------------------------------------------------
// Requirement: StayOnHome restarts run on a background queue
//   Scenario: c returns immediately while the restart runs in background
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn c_returns_input_control_while_restart_runs_in_background() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_c_nonblock");
    let title = "BG Nonblock C";
    let (_instance_id, _session_name, primary) = setup_cat_instance(&mut h, title);

    h.assert_screen_contains("Sort: Newest");

    // `c` enqueues the restart; `o` is the responsiveness probe sent right
    // behind it.
    h.send_keys("c");
    h.send_keys("o");

    // THEN the home view accepts further key input without waiting for the
    // restart: the sort label must flip well inside the pipeline's 12s
    // auto-confirm window. Today the event loop is frozen inside the restart
    // and the probe sits unprocessed -> RED.
    h.wait_for_timeout("Sort: Oldest", Duration::from_secs(3));

    // AND the session transitions through Restarting...
    h.wait_for(RESTARTING);

    // ...to Starting exactly as a synchronous restart would: the marker
    // clears once the worker's result is applied, the pane was really
    // respawned (its start command now carries the Cross Agent Team flag the
    // original launch did not have), and the persisted status is `starting`.
    h.wait_for_absent(RESTARTING, Duration::from_secs(25));
    wait_for_pane_start_command_contains(&h, &primary, CAT_FLAG, Duration::from_secs(10));
    wait_for_session_field(
        &h,
        title,
        "status",
        |status| status == Some("starting"),
        Duration::from_secs(10),
    );
}

// ---------------------------------------------------------------------------
// Requirement: StayOnHome restarts run on a background queue
//   Scenario: r on a dead session recovers in background
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn r_on_dead_session_recovers_in_background_and_stays_responsive() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_recover");
    let title = "BG Recover";
    let native = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1";

    let instance_id = add_and_start(&h, title);
    enable_cross_agent_team(&h, title);
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let project = h.project_path().to_str().unwrap().to_string();

    // Give slot 0 a real native id through the real capture path, so the
    // recovery relaunch is observable as `--resume <native>`.
    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    run_record_pane(&h, &primary, &instance_id, native, &project);

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.wait_for(title);
    wait_for_sql(
        &h,
        &format!(
            "SELECT native_session_id FROM agent_slot \
             WHERE instance_id='{instance_id}' AND slot=0;"
        ),
        native,
        Duration::from_secs(15),
    );
    force_cat_on_slots(&h, &instance_id);
    let old_slot0_pane = sqlite_query(
        &db_path(&h),
        &format!("SELECT tmux_pane FROM agent_slot WHERE instance_id='{instance_id}' AND slot=0;"),
    );

    // Cold start: the managed session dies, the home view flags the instance.
    h.kill_tmux_target(&session_name);
    assert!(
        !session_exists(&h, &session_name),
        "managed session must be dead after kill (cold-start precondition)"
    );
    h.wait_for("[recoverable]");

    // WHEN `r` recovers it; `o` probes responsiveness during the rebuild.
    h.send_keys("r");
    h.send_keys("o");

    // THEN the home view remains responsive while the rebuild runs on the
    // worker. Today the recovery (rebuild + relaunch + auto-confirm + settle)
    // runs synchronously on the event loop -> RED.
    h.wait_for_timeout("Sort: Oldest", Duration::from_secs(3));
    h.wait_for(RESTARTING);

    // AND the cold-start recovery really runs to completion in the
    // background: session rebuilt, slot 0 rebound to a fresh pane that
    // resumes from its own persisted native id, in-flight marker cleared.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !session_exists(&h, &session_name) {
        assert!(
            Instant::now() < deadline,
            "recovery never rebuilt the tmux session.\n\n--- Screen ---\n{}",
            h.capture_screen()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let new_slot0_pane = {
        let sql = format!(
            "SELECT tmux_pane FROM agent_slot WHERE instance_id='{instance_id}' AND slot=0;"
        );
        let start = Instant::now();
        loop {
            let got = sqlite_query(&db_path(&h), &sql);
            if !got.is_empty() && got != old_slot0_pane {
                break got;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "slot 0 was never rebound to a rebuilt pane (last={got:?}).\
                 \n\n--- Screen ---\n{}",
                h.capture_screen()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    wait_for_pane_start_command_contains(
        &h,
        &new_slot0_pane,
        &format!("--resume {native}"),
        Duration::from_secs(30),
    );
    h.wait_for_absent(RESTARTING, Duration::from_secs(30));
}

// ---------------------------------------------------------------------------
// Requirement: StayOnHome restarts run on a background queue
//   Scenario: Restarts of different sessions queue serially
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn restarts_of_two_sessions_queue_serially_and_both_show_restarting() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_serial");
    let project = h.project_path().to_str().unwrap().to_string();
    let fixtures = [
        ("Serial Restart A", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2"),
        ("Serial Restart B", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb2"),
    ];

    let mut by_title: std::collections::HashMap<&str, (String, String)> =
        std::collections::HashMap::new();
    for (title, native) in fixtures {
        let instance_id = add_and_start(&h, title);
        enable_cross_agent_team(&h, title);
        let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
        let primary = h.tmux_display_message(&session_name, "#{pane_id}");
        run_record_pane(&h, &primary, &instance_id, native, &project);
        by_title.insert(title, (instance_id, primary));
    }

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    for (title, native) in fixtures {
        h.wait_for(title);
        let (instance_id, _) = &by_title[title];
        wait_for_sql(
            &h,
            &format!(
                "SELECT native_session_id FROM agent_slot \
                 WHERE instance_id='{instance_id}' AND slot=0;"
            ),
            native,
            Duration::from_secs(15),
        );
        force_cat_on_slots(&h, instance_id);
    }

    // The cursor starts on the first displayed row and `j` moves to the
    // second; read the display order off the screen instead of assuming the
    // sort put A first.
    let screen = h.capture_screen();
    let pos_a = screen.find(fixtures[0].0).expect("A row on screen");
    let pos_b = screen.find(fixtures[1].0).expect("B row on screen");
    let (first, second) = if pos_a < pos_b {
        (fixtures[0], fixtures[1])
    } else {
        (fixtures[1], fixtures[0])
    };

    // WHEN `r` on the first session and immediately `r` on the second.
    h.send_keys("r");
    h.send_keys("j");
    h.send_keys("r");

    // THEN both instances show Restarting: the second (now selected) at once,
    // and the first when re-selected -- all well inside the first restart's
    // 12s window. Today the first `r` freezes the loop -> RED.
    h.wait_for_timeout(RESTARTING, Duration::from_secs(5));
    h.send_keys("k");
    h.wait_for_timeout(RESTARTING, Duration::from_secs(5));

    // AND the worker executes them one after the other: the first session's
    // pane is respawned with its own resume id while the second is still
    // queued (its pane must not have been touched yet), and the second then
    // completes with the same pipeline semantics.
    let (_, first_pane) = &by_title[first.0];
    let (_, second_pane) = &by_title[second.0];
    wait_for_pane_start_command_contains(
        &h,
        first_pane,
        &format!("--resume {}", first.1),
        Duration::from_secs(15),
    );
    let second_cmd = pane_start_command(&h, second_pane);
    assert!(
        !second_cmd.contains("--resume"),
        "the second restart must still be queued while the first occupies the \
         serial worker; its pane was already respawned: {second_cmd:?}"
    );
    wait_for_pane_start_command_contains(
        &h,
        second_pane,
        &format!("--resume {}", second.1),
        Duration::from_secs(40),
    );
}

// ---------------------------------------------------------------------------
// Requirement: In-flight restart gates conflicting operations
//   Scenario: Attach is rejected while restarting
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn attach_is_rejected_while_restart_is_in_flight() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_gate_attach");
    let (_instance_id, _session_name, _primary) = setup_cat_instance(&mut h, "BG Attach Gate");

    h.send_keys("r");
    // The in-flight marker must be visible before Enter means anything.
    // Today the sync restart never redraws -> RED here.
    h.wait_for(RESTARTING);

    h.send_keys("Enter");

    // THEN the system does not attach: the home view stays up for the whole
    // window and the restart is still in flight afterwards.
    assert_stays_on_screen(&h, "Agent of Empires", Duration::from_secs(3));
    h.assert_screen_contains(RESTARTING);
}

// ---------------------------------------------------------------------------
// Requirement: In-flight restart gates conflicting operations
//   Scenario: Delete is rejected while restarting
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn delete_is_rejected_while_restart_is_in_flight() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_gate_delete");
    let (_instance_id, _session_name, _primary) = setup_cat_instance(&mut h, "BG Delete Gate");

    h.send_keys("r");
    h.wait_for(RESTARTING);

    h.send_keys("d");

    // THEN no delete dialog opens for the in-flight session.
    assert_stays_absent(&h, "Delete Session", Duration::from_secs(3));
    h.assert_screen_contains(RESTARTING);
}

// ---------------------------------------------------------------------------
// Requirement: In-flight restart gates conflicting operations
//   Scenario: Duplicate restart is rejected while restarting
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn duplicate_restart_is_rejected_while_restart_is_in_flight() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_gate_duplicate");
    let (_instance_id, _session_name, primary) = setup_cat_instance(&mut h, "BG Duplicate Gate");

    h.send_keys("r");
    h.wait_for(RESTARTING);

    // All four restart keys are pressed while in flight; none may enqueue,
    // execute, or attach.
    h.send_keys("r");
    h.send_keys("c");
    h.send_keys("R");
    h.send_keys("C");

    // The first restart completes...
    h.wait_for_absent(RESTARTING, Duration::from_secs(25));

    // ...and nothing runs after it. A wrongly-queued duplicate would start
    // the moment the serial worker frees up: the pane would be respawned
    // again (new pane pid) and the row would re-enter Restarting. The buffered
    // uppercase variants would additionally attach and tear down the home
    // view. Watch the settled state long enough for any of that to show.
    let settled_pid = h.tmux_display_message(&primary, "#{pane_pid}");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let screen = h.capture_screen();
        assert!(
            !screen.contains(RESTARTING),
            "a second restart ran after the first completed; duplicate keys \
             were not rejected.\n\n--- Screen ---\n{screen}"
        );
        assert!(
            screen.contains("Agent of Empires"),
            "an uppercase restart variant attached after the in-flight window; \
             duplicate keys were not rejected.\n\n--- Screen ---\n{screen}"
        );
        let pid = h.tmux_display_message(&primary, "#{pane_pid}");
        assert_eq!(
            pid, settled_pid,
            "the agent pane was respawned again after the first restart \
             completed; a duplicate restart was enqueued"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

// ---------------------------------------------------------------------------
// Requirement: In-flight restart gates conflicting operations
//   Scenario: Operations are re-enabled after the result is applied
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn operations_reenable_after_restart_result_applies() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_reenable");
    let (_instance_id, session_name, _primary) =
        setup_cat_instance_outside_tmux_env(&mut h, "BG Reenable");

    h.send_keys("r");
    h.wait_for(RESTARTING);
    h.wait_for_absent(RESTARTING, Duration::from_secs(25));

    // Delete works again: the dialog opens and is dismissed.
    h.send_keys("d");
    h.wait_for("Delete Session");
    h.send_keys("Escape");
    h.wait_for_absent("Delete Session", Duration::from_secs(5));

    // Restart works again: a fresh in-flight window opens and closes.
    h.send_keys("r");
    h.wait_for(RESTARTING);
    h.wait_for_absent(RESTARTING, Duration::from_secs(25));

    // Attach works again: Enter leaves the home view for the session, and a
    // real tmux client ends up attached to the managed session. The home
    // view disappearing alone is NOT sufficient evidence: a FAILED attach
    // also blanks the pane for a sub-second window (the TUI leaves the
    // alternate screen around the attach call before redrawing), and a
    // lucky poll can mistake that flicker for success. An attached client
    // on the managed session is durable and immune to the flicker.
    h.send_keys("Enter");
    h.wait_for_absent("Agent of Empires", Duration::from_secs(30));
    wait_for_client_on_session(&h, &session_name, Duration::from_secs(10));
}

// ---------------------------------------------------------------------------
// Requirement: Restart results merge back into the instance
//   Scenario: Fresh restart commits its new conversation identity via the
//   result
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn fresh_background_restart_commits_new_identity_through_result() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_fresh_identity");
    let title = "BG Fresh Identity";
    let instance_id = add_and_start(&h, title);
    enable_cross_agent_team(&h, title);

    // A stale resume token that the fresh-identity transaction must clear.
    let mut sessions = read_sessions(&h);
    let session = sessions
        .as_array_mut()
        .expect("sessions array")
        .iter_mut()
        .find(|s| s["title"] == title)
        .expect("created session");
    session["resume_token"] = serde_json::Value::String("stale-resume-token-e2e".to_string());
    write_sessions(&h, &sessions);

    let old_id = session_field(&h, title, "agent_session_id")
        .filter(|id| !id.is_empty())
        .expect("precondition: the launch must have pre-allocated an agent_session_id");
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let primary = h.tmux_display_message(&session_name, "#{pane_id}");

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.wait_for(title);
    force_cat_on_slots(&h, &instance_id);

    h.send_keys("c");
    // Background evidence first: sync restarts never render this -> RED.
    h.wait_for(RESTARTING);
    h.wait_for_absent(RESTARTING, Duration::from_secs(25));

    // THEN the applied result carries the newly allocated agent_session_id
    // (persisted by the post-merge save) and the cleared resume_token.
    let new_id = wait_for_session_field(
        &h,
        title,
        "agent_session_id",
        |id| id.is_some_and(|id| !id.is_empty() && id != old_id),
        Duration::from_secs(10),
    )
    .expect("new agent_session_id");
    assert_eq!(
        session_field(&h, title, "resume_token"),
        None,
        "a fresh restart must clear the stale resume_token through the result merge"
    );

    // The identity the result committed is the one the pane was actually
    // launched with.
    wait_for_pane_start_command_contains(
        &h,
        &primary,
        &format!("--session-id {new_id}"),
        Duration::from_secs(10),
    );
}

// ---------------------------------------------------------------------------
// Requirement: Restart results merge back into the instance
//   Scenario: Per-pane errors surface after a background restart
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn per_pane_errors_surface_after_background_restart() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_pane_error");
    let title = "BG Pane Error";
    let instance_id = add_and_start(&h, title);
    enable_cross_agent_team(&h, title);
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let project = h.project_path().to_str().unwrap().to_string();

    // Two tracked claude panes through the real capture path.
    h.resize_window(&session_name, 220, 60);
    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    run_record_pane(
        &h,
        &primary,
        &instance_id,
        "cccccccc-cccc-4ccc-8ccc-ccccccccccc3",
        &project,
    );
    let split = h.split_window_get_pane(&session_name);
    run_record_pane(
        &h,
        &split,
        &instance_id,
        "dddddddd-dddd-4ddd-8ddd-ddddddddddd3",
        &project,
    );

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.wait_for(title);
    wait_for_sql(
        &h,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
        "2",
        Duration::from_secs(15),
    );
    force_cat_on_slots(&h, &instance_id);

    // Remove the sibling pane so its slot points at a pane that no longer
    // exists: that slot's respawn must fail while the primary's succeeds.
    let kill = tmux(&h, &["kill-pane", "-t", &split]);
    assert!(
        kill.status.success(),
        "kill-pane failed: {}",
        String::from_utf8_lossy(&kill.stderr)
    );
    assert!(
        session_exists(&h, &session_name),
        "precondition: the session must survive losing one of two panes"
    );
    let slot_count = sqlite_query(
        &db_path(&h),
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        slot_count, "2",
        "precondition: both slot rows must survive the pane kill, so the \
         restart fans out to a pane that is gone"
    );

    h.send_keys("r");
    // Background evidence first: sync restarts never render this -> RED.
    h.wait_for(RESTARTING);

    // THEN the instance does not remain in Restarting...
    h.wait_for_absent(RESTARTING, Duration::from_secs(25));

    // ...and the pane failure lands in last_error, rendered by the preview
    // panel for the selected instance. The message wraps in the panel, so
    // assert on fragments that survive wrapping.
    h.wait_for("Error:");
    h.wait_for("pane(s)");
}

// ---------------------------------------------------------------------------
// Requirement: In-flight state is protected from background refreshes
//   Scenario: Disk reload during an in-flight restart
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn disk_reload_preserves_restarting_state_and_gating() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("bg_restart_reload_preserve");
    let (_instance_id, _session_name, _primary) = setup_cat_instance(&mut h, "BG Reload Preserve");

    h.send_keys("r");
    h.wait_for(RESTARTING);

    // The periodic disk reload fires every 5s; holding the marker for 7s
    // spans at least one reload. The in-memory Restarting state must survive
    // it uninterrupted.
    assert_stays_on_screen(&h, RESTARTING, Duration::from_secs(7));

    // AND the instance remains gated after the reload.
    h.send_keys("d");
    assert_stays_absent(&h, "Delete Session", Duration::from_secs(2));
    h.assert_screen_contains(RESTARTING);
}

// ---------------------------------------------------------------------------
// DEFERRED scenarios (not generated -- see the create-test run output):
//   - background-restart / "Worker failure never wedges the instance": the
//     scenario's WHEN is a total pipeline failure INCLUDING a worker panic.
//     The binary exposes no fault-injection entry point to force a panic (or
//     a whole-pipeline failure) from outside: a killed sibling pane produces
//     a per-pane error (covered above), a killed primary pane on a
//     single-pane session kills the session and routes to recovery instead,
//     and a dead-but-remaining pane respawns successfully. The catch_unwind
//     half of the guarantee is unit-test territory (tasks 4.1: a panicking
//     worker still delivers an error result).
// ---------------------------------------------------------------------------
