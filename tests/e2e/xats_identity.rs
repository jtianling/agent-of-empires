//! Runtime coverage for the xats identity key AoE injects into Cross Agent Team
//! panes.
//!
//! End-to-end identity continuity cannot be asserted here: it needs the xats
//! daemon to resolve the key back to a team and name, and that half is a separate
//! project. What is verifiable on the AoE side is exactly what AoE promises --
//! the variable is present, its value survives a clean restart and a clean
//! recovery, and it is absent when the feature is off.

use serial_test::serial;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::harness::TuiTestHarness;

fn sessions_path(h: &TuiTestHarness) -> PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    }
}

fn read_sessions(h: &TuiTestHarness) -> Vec<serde_json::Value> {
    let content = std::fs::read_to_string(sessions_path(h)).expect("read sessions.json");
    serde_json::from_str(&content).expect("parse sessions.json")
}

/// Create a session and turn Cross Agent Team on for it. There is no CLI flag for
/// the option, so it is written straight into the store before the TUI loads it.
fn add_cross_agent_team_session(h: &TuiTestHarness, title: &str) -> String {
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

    let mut sessions = read_sessions(h);
    let session = sessions
        .iter_mut()
        .find(|s| s["title"] == title)
        .expect("created session");
    session["cross_agent_team"] = serde_json::Value::Bool(true);
    let id = session["id"].as_str().expect("session id").to_string();
    std::fs::write(
        sessions_path(h),
        serde_json::to_string_pretty(&sessions).unwrap(),
    )
    .expect("enable Cross Agent Team");

    let start = h.run_cli_in_tmux(&["session", "start", title]);
    assert!(
        start.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    id
}

fn stored_identity_key(h: &TuiTestHarness, id: &str) -> Option<String> {
    read_sessions(h)
        .iter()
        .find(|s| s["id"].as_str() == Some(id))
        .and_then(|s| s["xats_identity_key"].as_str())
        .map(str::to_string)
}

fn pane_start_command(h: &TuiTestHarness, session: &str) -> String {
    h.tmux_display_message(session, "#{pane_start_command}")
}

/// Pull the injected key out of a pane's start command. The value is shell
/// quoted, which is also what keeps it out of the argument list.
fn injected_key(command: &str) -> Option<String> {
    let rest = command.split_once("XATS_IDENTITY_KEY='")?.1;
    let value = rest.split_once('\'')?.0;
    (!value.is_empty()).then(|| value.to_string())
}

/// Wait until the pane's start command carries an identity key, then return it.
fn wait_for_injected_key(h: &TuiTestHarness, session: &str) -> String {
    let start = Instant::now();
    loop {
        if let Some(key) = injected_key(&pane_start_command(h, session)) {
            return key;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "no identity key was injected; pane command was {:?}",
            pane_start_command(h, session)
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn wait_for_key_change(h: &TuiTestHarness, session: &str, previous: &str) -> String {
    let start = Instant::now();
    loop {
        let command = pane_start_command(h, session);
        if let Some(key) = injected_key(&command) {
            if key != previous {
                return key;
            }
        }
        if start.elapsed() > Duration::from_secs(6) {
            return previous.to_string();
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Poll the home view until the instance is classified as recoverable. The
/// classification refreshes on the status-poller cadence, which needs more than
/// the harness's default screen-wait budget after a session is killed.
fn wait_for_recoverable(h: &TuiTestHarness) {
    let start = Instant::now();
    loop {
        if h.capture_screen().contains("[recoverable]") {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(40),
            "instance never became recoverable:\n{}",
            h.capture_screen()
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn db_path(h: &TuiTestHarness) -> PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/aoe.db")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/aoe.db")
    }
}

fn sqlite3_available() -> bool {
    std::process::Command::new("sqlite3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn sqlite_query(db: &Path, sql: &str) -> String {
    let out = std::process::Command::new("sqlite3")
        .arg(db)
        .arg(sql)
        .output()
        .expect("sqlite3");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
#[serial]
fn clean_restart_reuses_the_same_identity_key() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("xats_identity_restart");
    let id = add_cross_agent_team_session(&h, "Identity Restart");
    let session_name = agent_of_empires::tmux::Session::generate_name(&id, "Identity Restart");

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    // The session was started before the option was enabled, so the key is minted
    // by the first launch AoE performs with it on.
    h.send_keys("C");
    let first = wait_for_injected_key(&h, &session_name);
    assert_eq!(
        stored_identity_key(&h, &id).as_deref(),
        Some(first.as_str()),
        "the injected key must be the one persisted on the instance record"
    );

    h.send_keys("C");
    let second = wait_for_key_change(&h, &session_name, &first);
    assert_eq!(
        second, first,
        "a clean restart must reuse the identity key, not mint a new one"
    );
}

#[test]
#[serial]
fn identity_key_survives_a_clean_recovery() {
    crate::harness::require_tmux!();
    if !sqlite3_available() {
        eprintln!("skipping: sqlite3 not available");
        return;
    }

    let mut h = TuiTestHarness::new("xats_identity_recovery");
    let id = add_cross_agent_team_session(&h, "Identity Recovery");
    let session_name = agent_of_empires::tmux::Session::generate_name(&id, "Identity Recovery");

    h.spawn_tui();
    h.wait_for("Agent of Empires");

    h.send_keys("C");
    let before = wait_for_injected_key(&h, &session_name);

    // A durable slot is required for the instance to be recoverable at all.
    let db = db_path(&h);
    let now = 1;
    sqlite_query(
        &db,
        &format!(
            "INSERT OR REPLACE INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, tmux_pane, xats_identity_key, \
              last_seen_at) \
             VALUES ('{id}', 0, 'claude', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa0', '{}', '%0', \
              '', {now});",
            h.project_path().to_str().unwrap()
        ),
    );

    h.kill_tmux_target(&session_name);
    wait_for_recoverable(&h);

    h.send_keys("C");
    let after = wait_for_injected_key(&h, &session_name);
    assert_eq!(
        after, before,
        "clean recovery must relaunch the primary pane with its existing identity key"
    );
}

#[test]
#[serial]
fn no_identity_key_without_cross_agent_team() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("xats_identity_disabled");
    h.install_tool_stub("claude");
    let project = h.project_path();
    let add = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "No Identity",
        "-c",
        "claude",
    ]);
    assert!(add.status.success());
    let id = read_sessions(&h)
        .iter()
        .find(|s| s["title"] == "No Identity")
        .and_then(|s| s["id"].as_str())
        .expect("session id")
        .to_string();
    let start = h.run_cli_in_tmux(&["session", "start", "No Identity"]);
    assert!(start.status.success());
    let session_name = agent_of_empires::tmux::Session::generate_name(&id, "No Identity");

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.send_keys("C");

    // Give the restart the same window the enabled tests wait for, then prove
    // nothing was injected.
    std::thread::sleep(Duration::from_secs(2));
    let command = pane_start_command(&h, &session_name);
    assert!(
        !command.contains("XATS_IDENTITY_KEY"),
        "a session without Cross Agent Team must not carry an identity key, got {command:?}"
    );
    assert_eq!(stored_identity_key(&h, &id), None);
}
