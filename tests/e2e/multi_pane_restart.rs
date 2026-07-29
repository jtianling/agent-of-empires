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
        // A hook fires with its own agent's environment around it: Codex's
        // session id is `$CODEX_THREAD_ID`, not a stdin field, so a stdin-only
        // capture would simulate an invocation that does not occur. Read from
        // the registry so a new agent's source cannot go missing here.
        let source = agent_of_empires::agents::get_agent(agent)
            .and_then(|a| a.hook_config.as_ref())
            .map(|hooks| hooks.session_id_source);
        if let Some(agent_of_empires::agents::SessionIdSource::EnvVar(name)) = source {
            cmd.env(name, session_id);
        }
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

/// Add + start an instance whose primary agent is `tool` (claude/codex/gemini).
///
/// The instance tool must match the agent recorded for slot 0 in the test:
/// `R` builds the primary pane's resume command from `self.tool`
/// (`get_tool_command()`), so a mismatched tool -- or a `--cmd-override` that
/// replaces the tool with a shell -- would suppress the very `--resume <id>`
/// these tests assert on. A long-lived stub for `tool` is installed so the
/// started primary pane survives to be tracked and respawned.
fn add_and_start(h: &TuiTestHarness, title: &str, tool: &str) -> String {
    h.install_tool_stub(tool);
    let project = h.project_path();
    let add = h.run_cli(&["add", project.to_str().unwrap(), "-t", title, "-c", tool]);
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

fn instance_id_for(h: &TuiTestHarness, title: &str) -> String {
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
    // `R` triggers Action::RespawnAgentPane (Resume) for the selected instance.
    h.send_keys("R");
}

fn press_fresh_restart(h: &TuiTestHarness) {
    // `C` triggers Action::RespawnAgentPane (Fresh) for the selected instance.
    h.send_keys("C");
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
    let instance_id = add_and_start(&h, "Multi Pane Resume All", "claude");
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
    let instance_id = add_and_start(&h, "Claude Resume Cmd", "claude");
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
    let instance_id = add_and_start(&h, "Codex Resume Cmd", "codex");
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
    let instance_id = add_and_start(&h, "Single Slot Resume", "claude");
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
    let instance_id = add_and_start(&h, "No Slots Fallback", "claude");
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
    // launch command (here the claude tool), with no resume flag harvested from a
    // nonexistent slot.
    wait_for_pane_start_command_contains(&h, &primary, "claude");
    let cmd = pane_start_command(&h, &primary);
    assert!(
        !cmd.contains("--resume"),
        "no-slots fallback must restart fresh (no --resume), got start command: {:?}",
        cmd
    );
}

/// Restarting a shell instance that has no tracked slots must leave it alive.
///
/// The single-pane path kills the pane's process tree and then respawns into
/// the pane it just emptied. That works only while something holds the pane
/// open across the kill. An agent pane is created with `remain-on-exit` on; a
/// shell pane is created with it off, so the pane dies with its process -- and
/// for a single-pane session, the session dies with the pane. The restart then
/// has nothing left to respawn into.
///
/// The slot-based path had the same defect and was fixed in `f136e8f5`. This
/// path was not, and it is the one every untracked shell session takes.
#[test]
#[serial]
fn no_tracked_panes_restart_keeps_a_shell_instance_alive() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_shell_alive");
    // A shell instance the way a real one is written: the tool is a shell, so
    // the pane carries no `remain-on-exit`.
    h.install_tool_stub("shell");
    let project = h.project_path();
    let add = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "Shell Stays Alive",
        "-c",
        "shell",
        "--cmd-override",
        "/bin/sh",
    ]);
    assert!(
        add.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let start = h.run_cli_in_tmux(&["session", "start", "Shell Stays Alive"]);
    assert!(
        start.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let instance_id = instance_id_for(&h, "Shell Stays Alive");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Shell Stays Alive");

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    let slot_count = sqlite_query(
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        slot_count, "0",
        "precondition: the instance must have no tracked slots, which is what \
         sends the restart down the single-pane path"
    );

    press_restart(&h);

    // The session outliving its own restart is the whole assertion. Poll rather
    // than sample once: the kill and the respawn are not simultaneous, and a
    // session that is gone stays gone.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last_alive = true;
    while Instant::now() < deadline {
        last_alive = Command::new("tmux")
            .arg("-S")
            .arg(h.tmux_socket_path())
            .args(["has-session", "-t", &session_name])
            .output()
            .expect("failed to run tmux has-session")
            .status
            .success();
        if !last_alive {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        last_alive,
        "restarting an untracked shell instance destroyed its session: the pane \
         was killed without anything holding it open, and a single-pane session \
         dies with its pane.\n\n--- Screen ---\n{}",
        h.capture_screen()
    );
}

/// The pane a user handed to a different agent must come back as that agent.
///
/// The fallback above rebuilds from the instance's tool, which is right only
/// while the pane still runs it. A user who exits Claude in the pane and runs
/// Codex there leaves no `agent_slot` row to say so -- Codex installs no hook,
/// so nothing records the pane -- and the restart then does not restart the
/// pane, it replaces the agent in it.
#[test]
#[serial]
fn no_tracked_panes_restarts_the_agent_the_pane_actually_runs() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_hijacked");
    let instance_id = add_and_start(&h, "Hijacked Pane", "claude");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Hijacked Pane");

    // tmux names a pane's process from the kernel, so this stub has to be a
    // real binary: a shell script called `codex` reports as `bash` and the pane
    // would look like it runs nothing in particular.
    let Some(codex_bin) = h.install_native_stub("codex") else {
        eprintln!("Skipping test: no C compiler to build a native stub");
        return;
    };

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    let slot_count = sqlite_query(
        &db,
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{instance_id}';"),
    );
    assert_eq!(
        slot_count, "0",
        "precondition: the pane must be untracked, which is what makes the \
         instance's tool the only thing the restart has to go on"
    );

    // The hand-off: the pane now runs Codex, and nothing recorded it.
    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    let respawn = Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args(["respawn-pane", "-k", "-t", &primary])
        .arg(codex_bin.to_str().unwrap())
        .output()
        .expect("failed to run tmux respawn-pane");
    assert!(
        respawn.status.success(),
        "respawn-pane failed: {}",
        String::from_utf8_lossy(&respawn.stderr)
    );

    // And it must actually be running under that name, or this test proves
    // nothing: the guard reads the pane's process, and a stub that died or was
    // never built leaves the pane looking like a plain shell.
    let start = Instant::now();
    loop {
        if h.tmux_display_message(&primary, "#{pane_current_command}") == "codex" {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the pane never came up running codex; nothing here is observable"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    press_restart(&h);

    wait_for_pane_start_command_contains(&h, &primary, "codex");
    let cmd = pane_start_command(&h, &primary);
    assert!(
        !cmd.contains("claude"),
        "the restart must not replace the pane's agent with the instance's tool, \
         got start command: {cmd:?}"
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
    let instance_id = add_and_start(&h, "Failure Isolation", "gemini");
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
    let instance_id = add_and_start(&h, "Empty Id Fresh", "claude");
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
// Requirement: `r` restarts every tracked pane fresh (no resume flag)
// Requirement: `R` still resumes; `e` opens the rename dialog
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn c_restarts_every_tracked_pane_clean_without_resume() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_fresh_restart");
    let instance_id = add_and_start(&h, "Multi Pane Fresh", "claude");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Multi Pane Fresh");

    // Three tracked claude panes, each with a valid, distinct native_session_id
    // that WOULD resume under `R`.
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

    press_fresh_restart(&h);

    // Every tracked pane must be respawned with the fresh claude command and NO
    // resume flag, even though each slot has a valid resume id.
    for pane in &panes {
        wait_for_pane_start_command_contains(&h, pane, "claude");
        let cmd = pane_start_command(&h, pane);
        assert!(
            !cmd.contains("--resume"),
            "fresh restart (`r`) must carry no resume flag for pane {}, got: {:?}",
            pane,
            cmd
        );
    }
}

#[test]
#[serial]
fn shift_r_still_resumes_after_rebind() {
    crate::harness::require_tmux!();
    require_sqlite3!();

    let mut h = TuiTestHarness::new("multi_pane_resume_after_rebind");
    let instance_id = add_and_start(&h, "Resume After Rebind", "claude");
    let db = db_path(&h);
    let session_name =
        agent_of_empires::tmux::Session::generate_name(&instance_id, "Resume After Rebind");

    let panes = establish_tracked_panes(
        &mut h,
        &instance_id,
        &session_name,
        &[("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11", None)],
        &db,
    );

    press_restart(&h);

    // `R` (Shift) keeps resuming from the persisted id.
    wait_for_pane_start_command_contains(
        &h,
        &panes[0],
        "claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11",
    );
}

#[test]
#[serial]
fn e_opens_the_rename_dialog() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("multi_pane_edit_rename");
    let _instance_id = add_and_start(&h, "Edit Rename Target", "claude");

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.wait_for("Edit Rename Target");

    // `e` opens the session rename/edit dialog.
    h.send_keys("e");
    h.wait_for("Edit Session");
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
