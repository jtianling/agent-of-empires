//! E2E tests for the `session-agent-switch` capability.
//!
//! `a` on the home view opens the switch-agent dialog; confirming hands the
//! session to the other agent and restarts it fresh in the background. The
//! switch is observed through the relaunched pane's `#{pane_start_command}`,
//! `sessions.json` and the durable slot row, whose xats identity key must
//! survive. All tmux traffic goes through the harness's private socket.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serial_test::serial;

use crate::harness::TuiTestHarness;

fn profile_dir(h: &TuiTestHarness) -> PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default")
    } else {
        h.home_path().join(".agent-of-empires/profiles/default")
    }
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
        "sqlite3 query failed for {:?}: {}",
        sql,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn session_field(h: &TuiTestHarness, title: &str, field: &str) -> Option<String> {
    let content =
        std::fs::read_to_string(profile_dir(h).join("sessions.json")).expect("sessions.json");
    let sessions: serde_json::Value = serde_json::from_str(&content).expect("parse sessions");
    sessions
        .as_array()?
        .iter()
        .find(|s| s["title"].as_str() == Some(title))
        .and_then(|s| s[field].as_str())
        .map(str::to_string)
}

fn wait_until(h: &TuiTestHarness, what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        if start.elapsed() > timeout {
            panic!(
                "Timed out waiting for {what}.\n\n--- Screen ---\n{}",
                h.capture_screen()
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn start_command_runs(h: &TuiTestHarness, pane: &str, agent: &str) -> bool {
    let command = h.tmux_display_message(pane, "#{pane_start_command}");
    command
        .split_whitespace()
        .any(|word| word.trim_matches(|c| c == '\'' || c == '"') == agent)
}

fn slot_row(h: &TuiTestHarness, instance_id: &str) -> String {
    sqlite_query(
        &profile_dir(h).join("aoe.db"),
        &format!(
            "SELECT agent || '|' || xats_identity_key FROM agent_slot \
             WHERE instance_id='{instance_id}' AND slot=0;"
        ),
    )
}

#[test]
#[serial]
fn a_switches_between_claude_and_codex_and_keeps_the_identity_key() {
    crate::harness::require_tmux!();
    if Command::new("sqlite3").arg("--version").output().is_err() {
        eprintln!("Skipping test: sqlite3 CLI not available");
        return;
    }

    let mut h = TuiTestHarness::new("agent_switch_roundtrip");
    h.install_tool_stub("claude");
    h.install_tool_stub("codex");
    let title = "Switch Roundtrip";
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
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let start = h.run_cli_in_tmux(&["session", "start", title]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let instance_id = session_field(&h, title, "id").expect("session id");
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, title);
    let primary = h.tmux_display_message(&session_name, "#{pane_id}");
    assert!(start_command_runs(&h, &primary, "claude"));

    let db = profile_dir(&h).join("aoe.db");
    sqlite_query(
        &db,
        &format!(
            "UPDATE agent_slot SET xats_identity_key='e2e-identity-key' \
             WHERE instance_id='{instance_id}' AND slot=0;"
        ),
    );
    assert_eq!(slot_row(&h, &instance_id), "claude|e2e-identity-key");

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.wait_for(title);

    h.send_keys("a");
    h.wait_for("Switch Agent");
    h.assert_screen_contains("codex");
    h.send_keys("Enter");

    wait_until(
        &h,
        "the pane to relaunch as codex",
        Duration::from_secs(30),
        || start_command_runs(&h, &primary, "codex"),
    );
    assert_eq!(session_field(&h, title, "tool").as_deref(), Some("codex"));
    assert_eq!(slot_row(&h, &instance_id), "codex|e2e-identity-key");
    h.assert_screen_contains("Agent of Empires");

    wait_until(&h, "the restart to settle", Duration::from_secs(30), || {
        !h.capture_screen().contains("Restarting...")
    });
    h.send_keys("a");
    h.wait_for("Switch Agent");
    h.send_keys("Enter");

    wait_until(
        &h,
        "the pane to relaunch as claude",
        Duration::from_secs(30),
        || start_command_runs(&h, &primary, "claude"),
    );
    assert_eq!(session_field(&h, title, "tool").as_deref(), Some("claude"));
    assert_eq!(slot_row(&h, &instance_id), "claude|e2e-identity-key");
}

#[test]
#[serial]
fn escape_leaves_the_session_on_its_agent() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("agent_switch_cancel");
    let title = "Switch Cancel";
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
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.wait_for(title);
    h.send_keys("a");
    h.wait_for("Switch Agent");
    h.send_keys("Escape");
    h.wait_for_absent("Switch Agent", Duration::from_secs(5));

    assert_eq!(session_field(&h, title, "tool").as_deref(), Some("claude"));
}
