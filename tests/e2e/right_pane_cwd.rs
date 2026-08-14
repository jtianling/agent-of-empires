//! e2e coverage for per-pane working directories.
//!
//! The defect these guard is that a right pane's directory used to be the
//! session's for both the split and the durable slot record. The split alone
//! looks correct: the pane starts in the right place and only comes back in the
//! wrong one after a restart, with nothing on screen connecting the two events.
//! So each directory is read from the pane itself, and the restart case is
//! asserted separately.

use std::process::Command;
use std::time::{Duration, Instant};

use serial_test::serial;

use crate::harness::TuiTestHarness;

fn config_dir(h: &TuiTestHarness) -> std::path::PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path().join(".config").join("agent-of-empires")
    } else {
        h.home_path().join(".agent-of-empires")
    }
}

fn write_shell_default_config(h: &TuiTestHarness) {
    let config_content = format!(
        r#"[updates]
check_enabled = false

[app_state]
has_seen_welcome = true
last_seen_version = "{}"

[session]
default_tool = "shell"
"#,
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(config_dir(h).join("config.toml"), config_content).expect("write config.toml");
}

fn sessions_json(h: &TuiTestHarness) -> Option<serde_json::Value> {
    let path = config_dir(h)
        .join("profiles")
        .join("default")
        .join("sessions.json");
    let content = std::fs::read_to_string(&path).ok()?;
    Some(serde_json::from_str(&content).expect("invalid sessions JSON"))
}

fn instance_id_for(h: &TuiTestHarness, title: &str) -> String {
    let sessions = sessions_json(h).expect("read sessions.json");
    sessions
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|session| session["title"].as_str() == Some(title))
        .and_then(|session| session["id"].as_str())
        .expect("session id")
        .to_string()
}

/// Create a session running `tool` at `dir` and start it, without going through
/// the TUI. A stub for `tool` is installed first so the pane it launches stays
/// alive long enough to be restarted.
fn add_and_start(h: &TuiTestHarness, title: &str, dir: &str, tool: &str) -> String {
    if tool != "shell" {
        h.install_tool_stub(tool);
    }
    let add = h.run_cli(&["add", dir, "-t", title, "-c", tool]);
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
    let id = instance_id_for(h, title);
    agent_of_empires::tmux::Session::generate_name(&id, title)
}

fn wait_for_tmux_session_name(h: &TuiTestHarness, title: &str) -> String {
    let start = Instant::now();
    while start.elapsed() <= Duration::from_secs(10) {
        if let Some(id) = sessions_json(h).and_then(|sessions| {
            sessions.as_array().and_then(|sessions| {
                sessions.iter().find_map(|session| {
                    (session["title"].as_str() == Some(title))
                        .then(|| session["id"].as_str().expect("session id").to_string())
                })
            })
        }) {
            return agent_of_empires::tmux::Session::generate_name(&id, title);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    panic!(
        "Timed out waiting for session {} to be created.\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
        title,
        h.capture_screen()
    );
}

fn wait_for_pane_count(h: &TuiTestHarness, session_name: &str, expected: &str) {
    let start = Instant::now();
    while start.elapsed() <= Duration::from_secs(15) {
        if h.tmux_display_message(session_name, "#{window_panes}") == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "Timed out waiting for {} panes in {}, saw {}",
        expected,
        session_name,
        h.tmux_display_message(session_name, "#{window_panes}")
    );
}

/// The directory the pane's own shell reports, not the one the dialog was told.
fn pane_reported_cwd(h: &TuiTestHarness, target: &str) -> String {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() <= Duration::from_secs(10) {
        last = h.tmux_display_message(target, "#{pane_current_path}");
        if !last.is_empty() {
            return last;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    last
}

fn wait_for_pane_cwd(h: &TuiTestHarness, target: &str, expected: &str) {
    let start = Instant::now();
    while start.elapsed() <= Duration::from_secs(15) {
        if pane_reported_cwd(h, target) == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "Timed out waiting for pane {} to report {}, saw {}",
        target,
        expected,
        pane_reported_cwd(h, target)
    );
}

fn pane_id_at(h: &TuiTestHarness, session_name: &str, index: usize) -> String {
    h.tmux_display_message(&format!("{}:.{}", session_name, index), "#{pane_id}")
}

/// Wait until a pane is running a different process than `before`.
///
/// This is how a respawn is observed from outside: `respawn-pane -k` reuses the
/// pane id, so the pane's own process is the only thing that changes. Without
/// it a directory assertion can run before the restart has landed and pass on
/// the pre-restart state, which is precisely the state the restart is supposed
/// to be judged against.
fn wait_for_pane_respawn(h: &TuiTestHarness, pane_id: &str, before: &str) {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() <= Duration::from_secs(20) {
        last = h.tmux_display_message(pane_id, "#{pane_pid}");
        if !last.is_empty() && last != before {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    panic!(
        "Timed out waiting for pane {} to be respawned; its process stayed {} (last read {})",
        pane_id, before, last
    );
}

fn make_dir(h: &TuiTestHarness, name: &str) -> String {
    let dir = h.home_path().join(name);
    std::fs::create_dir_all(&dir).expect("create dir");
    dir.canonicalize()
        .expect("canonicalize dir")
        .display()
        .to_string()
}

fn right_pane_line(screen: &str) -> &str {
    screen
        .lines()
        .find(|line| line.contains("Right Pane Agent:"))
        .unwrap_or("")
}

/// Drive the new session dialog to a shell right pane in `right_pane_dir`.
fn create_session_with_right_pane(
    h: &TuiTestHarness,
    title: &str,
    session_dir: &str,
    right_pane_dir: &str,
) {
    h.send_keys("n");
    h.wait_for("Title");
    h.type_text(title);

    for _ in 0..3 {
        h.send_keys("Tab");
    }
    for _ in 0..128 {
        h.send_keys("BSpace");
    }
    h.type_text(session_dir);

    for _ in 0..2 {
        h.send_keys("Tab");
    }

    for _ in 0..32 {
        if right_pane_line(&h.capture_screen()).contains("● shell") {
            break;
        }
        h.send_keys("Right");
        // One key per redraw: capturing before the TUI has caught up means
        // pressing against a stale screen, and the surplus presses land after
        // the loop is satisfied.
        std::thread::sleep(Duration::from_millis(120));
    }
    assert!(
        right_pane_line(&h.capture_screen()).contains("● shell"),
        "right pane tool should be shell\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
        h.capture_screen()
    );

    h.wait_for("Right Pane Path");
    h.send_keys("Tab");
    h.type_text(right_pane_dir);
    h.send_keys("Enter");
}

#[test]
#[serial]
fn right_pane_starts_in_its_own_directory() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("right_pane_own_cwd");
    write_shell_default_config(&h);
    let session_dir = make_dir(&h, "session-dir");
    let pane_dir = make_dir(&h, "pane-dir");

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    create_session_with_right_pane(&h, "Own Cwd", &session_dir, &pane_dir);

    let session_name = wait_for_tmux_session_name(&h, "Own Cwd");
    wait_for_pane_count(&h, &session_name, "2");

    wait_for_pane_cwd(&h, &format!("{}:.1", session_name), &pane_dir);
    assert_eq!(
        pane_reported_cwd(&h, &format!("{}:.0", session_name)),
        session_dir,
        "the left pane still starts in the session's directory"
    );
}

/// The restart is where the defect was observable: the split alone looks
/// correct, and only the slot record decides where each pane comes back.
///
/// Both panes run agents here so the test remains focused on separate agent
/// directories. Managed shell lifecycle coverage lives in `codex_xats`.
#[test]
#[serial]
fn a_restart_returns_both_panes_to_their_own_directories() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("right_pane_restart_cwd");
    write_shell_default_config(&h);
    let session_dir = make_dir(&h, "session-dir");
    let pane_dir = make_dir(&h, "pane-dir");

    let session_name = add_and_start(&h, "Restart Cwd", &session_dir, "claude");
    let added = h.run_cli(&[
        "session",
        "add-agent-pane",
        "Restart Cwd",
        "--tool",
        "claude",
        "--path",
        &pane_dir,
    ]);
    assert!(
        added.status.success(),
        "add-agent-pane failed: {}",
        String::from_utf8_lossy(&added.stderr)
    );

    wait_for_pane_count(&h, &session_name, "2");
    let primary = pane_id_at(&h, &session_name, 0);
    let extra = pane_id_at(&h, &session_name, 1);
    wait_for_pane_cwd(&h, &extra, &pane_dir);
    wait_for_pane_cwd(&h, &primary, &session_dir);

    let primary_process = h.tmux_display_message(&primary, "#{pane_pid}");
    let extra_process = h.tmux_display_message(&extra, "#{pane_pid}");

    // `c` is a clean restart of every tracked pane that stays on the home list.
    h.spawn_tui();
    h.wait_for("Restart Cwd");
    h.send_keys("c");

    // Both panes must be observed coming back before their directories mean
    // anything. Waiting on the pane count alone would be satisfied by the
    // pre-restart panes, which never moved.
    wait_for_pane_respawn(&h, &primary, &primary_process);
    wait_for_pane_respawn(&h, &extra, &extra_process);

    wait_for_pane_cwd(&h, &extra, &pane_dir);
    wait_for_pane_cwd(&h, &primary, &session_dir);
}

/// A managed shell pane is slot-recorded, so the resume path reaches a slot
/// whose recorded agent is `shell`.
/// The agent registry's binary for `shell` is the literal string `shell`, which
/// names no program: a resume that built the command from the registry would
/// respawn the pane into a command that does not exist. So this asserts the
/// pane comes back alive, in its own directory, running a real shell.
#[test]
#[serial]
fn a_restart_relaunches_a_custom_cwd_shell_pane_as_a_real_shell() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("right_pane_shell_restart");
    write_shell_default_config(&h);
    let session_dir = make_dir(&h, "session-dir");
    let pane_dir = make_dir(&h, "pane-dir");

    let session_name = add_and_start(&h, "Shell Restart", &session_dir, "claude");
    let added = h.run_cli(&[
        "session",
        "add-agent-pane",
        "Shell Restart",
        "--tool",
        "shell",
        "--path",
        &pane_dir,
    ]);
    assert!(
        added.status.success(),
        "add-agent-pane failed: {}",
        String::from_utf8_lossy(&added.stderr)
    );

    wait_for_pane_count(&h, &session_name, "2");
    let extra = pane_id_at(&h, &session_name, 1);
    wait_for_pane_cwd(&h, &extra, &pane_dir);
    let extra_process = h.tmux_display_message(&extra, "#{pane_pid}");

    h.spawn_tui();
    h.wait_for("Shell Restart");
    h.send_keys("c");

    // Reaching the fan-out at all is half the assertion: a shell pane that was
    // not slot-recorded would be skipped entirely and its process would never
    // change, so this fails rather than passing on the pre-restart state.
    wait_for_pane_respawn(&h, &extra, &extra_process);
    wait_for_pane_cwd(&h, &extra, &pane_dir);

    // A pane respawned into a nonexistent `shell` command exits immediately.
    assert_eq!(
        h.tmux_display_message(&extra, "#{pane_dead}"),
        "0",
        "the relaunched shell pane must still be alive"
    );
    let start_command = h.tmux_display_message(&extra, "#{pane_start_command}");
    assert!(
        !start_command.split_whitespace().any(|word| word == "shell"),
        "the pane must be relaunched as a real shell, not the registry's literal \
         `shell` binary; its start command was: {start_command}"
    );
}

#[test]
#[serial]
fn percent_adds_a_pane_with_the_chosen_agent_and_directory() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("right_pane_percent");
    write_shell_default_config(&h);
    let session_dir = make_dir(&h, "session-dir");
    let pane_dir = make_dir(&h, "pane-dir");

    // The added pane must run an agent the session does not, or the test passes
    // just as well when the dialog's agent selection is ignored entirely.
    h.install_tool_stub("claude");

    let session_name = add_and_start(&h, "Percent", &session_dir, "shell");
    wait_for_pane_count(&h, &session_name, "1");

    h.spawn_tui();
    h.wait_for("Percent");

    h.send_keys("%");
    h.wait_for("Add Agent Pane");
    select_add_pane_agent(&h, "claude");
    h.send_keys("Tab");
    h.type_text(&pane_dir);
    h.send_keys("Enter");

    wait_for_pane_count(&h, &session_name, "2");
    wait_for_pane_cwd(&h, &format!("{}:.1", session_name), &pane_dir);
    // The agent that was chosen, read from what tmux recorded as the pane's
    // start command rather than from the dialog.
    let start_command =
        h.tmux_display_message(&format!("{}:.1", session_name), "#{pane_start_command}");
    assert!(
        start_command.contains("claude"),
        "the added pane must run the chosen agent, its start command was: {start_command}"
    );
    // The pane beside it keeps the session's directory: `%` adds a peer, it
    // does not move the session.
    wait_for_pane_cwd(&h, &format!("{}:.0", session_name), &session_dir);

    // The spec has `%` attach once the pane is up. Without this the test passes
    // for a flow that created the pane and left the user on the home list.
    wait_for_session_attached(&h, &session_name);
}

/// Cycle the add-pane dialog's agent selector until `agent` is selected.
///
/// The offered list is whatever the machine detects, so the number of presses
/// cannot be hardcoded; the selection is read back off the screen instead.
fn select_add_pane_agent(h: &TuiTestHarness, agent: &str) {
    let selected = format!("● {agent}");
    for _ in 0..32 {
        let screen = h.capture_screen();
        let agent_line = screen
            .lines()
            .find(|line| line.contains("Agent:"))
            .unwrap_or("");
        if agent_line.contains(&selected) {
            return;
        }
        h.send_keys("Right");
        std::thread::sleep(Duration::from_millis(120));
    }
    panic!(
        "Could not select agent {} in the add-pane dialog\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
        agent,
        h.capture_screen()
    );
}

/// Wait until the managed session has a client attached to it.
fn wait_for_session_attached(h: &TuiTestHarness, session_name: &str) {
    let start = Instant::now();
    while start.elapsed() <= Duration::from_secs(15) {
        if h.tmux_display_message(session_name, "#{session_attached}") != "0" {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "Timed out waiting for {} to be attached, saw {}\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
        session_name,
        h.tmux_display_message(session_name, "#{session_attached}"),
        h.capture_screen()
    );
}

#[test]
#[serial]
fn cli_add_agent_pane_accepts_a_tool_and_a_path() {
    crate::harness::require_tmux!();

    let h = TuiTestHarness::new("right_pane_cli_add");
    write_shell_default_config(&h);
    let session_dir = make_dir(&h, "session-dir");
    let pane_dir = make_dir(&h, "pane-dir");

    let session_name = add_and_start(&h, "CliAdd", &session_dir, "shell");
    wait_for_pane_count(&h, &session_name, "1");

    let output = h.run_cli(&[
        "session",
        "add-agent-pane",
        "CliAdd",
        "--tool",
        "shell",
        "--path",
        &pane_dir,
    ]);
    assert!(
        output.status.success(),
        "add-agent-pane failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    wait_for_pane_count(&h, &session_name, "2");
    wait_for_pane_cwd(&h, &format!("{}:.1", session_name), &pane_dir);
}

#[test]
#[serial]
fn cli_add_agent_pane_rejects_a_missing_path() {
    crate::harness::require_tmux!();

    let h = TuiTestHarness::new("right_pane_cli_bad_path");
    write_shell_default_config(&h);
    let session_dir = make_dir(&h, "session-dir");

    add_and_start(&h, "BadPath", &session_dir, "shell");

    let missing = h.home_path().join("does-not-exist");
    let output = h.run_cli(&[
        "session",
        "add-agent-pane",
        "BadPath",
        "--path",
        missing.to_str().expect("path utf-8"),
    ]);

    assert!(!output.status.success(), "a missing directory must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Only the directory reason is accepted. The session was started above, so
    // allowing "not running" here would let this pass for a reason that has
    // nothing to do with the path, which is the failure it exists to catch.
    assert!(
        stderr.contains("Working directory not found"),
        "the refusal must name the directory, got: {stderr}"
    );
}

/// An unrecognized agent name must be refused at the CLI boundary. A sandboxed
/// session's extra pane falls back to a plain shell for an agent it does not
/// know, so a typo would launch bash and then persist the typo as the slot's
/// agent, taking every later restart of that pane with it.
#[test]
#[serial]
fn cli_add_agent_pane_rejects_an_unknown_tool() {
    crate::harness::require_tmux!();

    let h = TuiTestHarness::new("right_pane_cli_bad_tool");
    write_shell_default_config(&h);
    let session_dir = make_dir(&h, "session-dir");

    let session_name = add_and_start(&h, "BadTool", &session_dir, "shell");
    wait_for_pane_count(&h, &session_name, "1");

    let output = h.run_cli(&[
        "session",
        "add-agent-pane",
        "BadTool",
        "--tool",
        "definitely-not-an-agent",
    ]);

    assert!(!output.status.success(), "an unknown agent must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Unknown agent"),
        "the refusal must name the cause, got: {stderr}"
    );
    // The refusal must be total: no pane, so nothing to clean up and no slot
    // recorded against a name no registry entry holds.
    wait_for_pane_count(&h, &session_name, "1");
}

/// The tmux `%` binding inside an attached session is a different action from
/// the home screen's, and this change must not have moved it.
#[test]
#[serial]
fn prefix_percent_still_pins_a_hand_made_split_to_the_project_path() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("right_pane_prefix_percent");
    write_shell_default_config(&h);
    let session_dir = make_dir(&h, "session-dir");

    let session_name = add_and_start(&h, "PrefixPct", &session_dir, "shell");
    wait_for_pane_count(&h, &session_name, "1");

    // The binding is installed on attach, so the TUI has to reach that path.
    h.spawn_tui();
    h.wait_for("PrefixPct");
    h.send_keys("Enter");

    let start = Instant::now();
    let mut text = String::new();
    while start.elapsed() <= Duration::from_secs(15) {
        let binding = Command::new("tmux")
            .arg("-S")
            .arg(h.tmux_socket_path())
            .args(["list-keys", "-T", "prefix", "%"])
            .output()
            .expect("list-keys");
        text = String::from_utf8_lossy(&binding.stdout).to_string();
        if text.contains("@aoe_project_path") {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    panic!("prefix + % must keep pinning to the project path, got: {text}");
}
