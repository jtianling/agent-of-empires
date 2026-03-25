use serial_test::serial;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::harness::{require_tmux, TuiTestHarness};

fn create_profile(h: &TuiTestHarness, name: &str) {
    let config_dir = if cfg!(target_os = "linux") {
        h.home_path().join(".config").join("agent-of-empires")
    } else {
        h.home_path().join(".agent-of-empires")
    };
    std::fs::create_dir_all(config_dir.join("profiles").join(name)).expect("create profile dir");
}

fn read_sessions_json(h: &TuiTestHarness, profile: &str) -> serde_json::Value {
    let sessions_path = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles")
            .join(profile)
            .join("sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles")
            .join(profile)
            .join("sessions.json")
    };

    let content = std::fs::read_to_string(&sessions_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", sessions_path.display(), e));
    serde_json::from_str(&content).expect("invalid sessions JSON")
}

fn tmux_has_session(h: &TuiTestHarness, session_name: &str) -> bool {
    Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args(["has-session", "-t", session_name])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn wait_for_tmux_session_state(h: &TuiTestHarness, session_name: &str, exists: bool) {
    let start = Instant::now();
    while start.elapsed() <= Duration::from_secs(10) {
        if tmux_has_session(h, session_name) == exists {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    panic!(
        "timed out waiting for tmux session {} existence to become {}",
        session_name, exists
    );
}

#[test]
#[serial]
fn test_tui_rename_updates_tmux_session_name_without_killing_shell() {
    require_tmux!();

    let mut h = TuiTestHarness::new("tui_rename_same_profile");
    let project = h.project_path();

    let add_output = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "Old Title",
        "--cmd-override",
        "sh",
    ]);
    assert!(
        add_output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let start_output = h.run_cli_in_tmux(&["session", "start", "Old Title"]);
    assert!(
        start_output.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start_output.stderr)
    );

    let sessions = read_sessions_json(&h, "default");
    let session = sessions
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|session| session["title"].as_str() == Some("Old Title"))
        .expect("old title session");
    let session_id = session["id"].as_str().expect("session id");
    let old_tmux_name = agent_of_empires::tmux::Session::generate_name(session_id, "Old Title");
    let new_tmux_name = agent_of_empires::tmux::Session::generate_name(session_id, "New Title");

    wait_for_tmux_session_state(&h, &old_tmux_name, true);

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.wait_for("Old Title");
    h.send_keys("r");
    h.wait_for("Edit Session");
    h.type_text("New Title");
    h.send_keys("Enter");
    h.wait_for("New Title");

    wait_for_tmux_session_state(&h, &old_tmux_name, false);
    wait_for_tmux_session_state(&h, &new_tmux_name, true);
    assert_eq!(h.tmux_display_message(&new_tmux_name, "#{pane_dead}"), "0");

    h.kill_tmux_target(&new_tmux_name);
}

#[test]
#[serial]
fn test_tui_cross_profile_rename_updates_tmux_session_name_without_killing_shell() {
    require_tmux!();

    let mut h = TuiTestHarness::new("tui_rename_cross_profile");
    let project = h.project_path();
    create_profile(&h, "work");

    let add_output = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "Default Title",
        "--cmd-override",
        "sh",
    ]);
    assert!(
        add_output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let start_output = h.run_cli_in_tmux(&["session", "start", "Default Title"]);
    assert!(
        start_output.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start_output.stderr)
    );

    let default_sessions = read_sessions_json(&h, "default");
    let default_session = default_sessions
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|session| session["title"].as_str() == Some("Default Title"))
        .expect("default session");
    let session_id = default_session["id"].as_str().expect("session id");
    let old_tmux_name = agent_of_empires::tmux::Session::generate_name(session_id, "Default Title");
    let new_tmux_name = agent_of_empires::tmux::Session::generate_name(session_id, "Moved Title");

    wait_for_tmux_session_state(&h, &old_tmux_name, true);

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.wait_for("Default Title");
    h.send_keys("r");
    h.wait_for("Edit Session");
    h.type_text("Moved Title");
    h.send_keys("Down");
    h.send_keys("Down");
    h.send_keys("Right");
    h.send_keys("Enter");
    h.wait_for("No sessions yet");

    wait_for_tmux_session_state(&h, &old_tmux_name, false);
    wait_for_tmux_session_state(&h, &new_tmux_name, true);
    assert_eq!(h.tmux_display_message(&new_tmux_name, "#{pane_dead}"), "0");

    let default_sessions = read_sessions_json(&h, "default");
    assert!(
        default_sessions
            .as_array()
            .expect("sessions array")
            .iter()
            .all(|session| session["id"].as_str() != Some(session_id)),
        "session should be removed from default profile"
    );

    let work_sessions = read_sessions_json(&h, "work");
    let moved_session = work_sessions
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|session| session["id"].as_str() == Some(session_id))
        .expect("moved session");
    assert_eq!(moved_session["title"].as_str(), Some("Moved Title"));

    h.kill_tmux_target(&new_tmux_name);
}
