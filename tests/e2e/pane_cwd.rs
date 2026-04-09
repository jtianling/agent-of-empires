use std::process::Command;
use std::time::Duration;

use serial_test::serial;

use crate::harness::TuiTestHarness;

fn write_shell_default_config(h: &TuiTestHarness) {
    let config_dir = if cfg!(target_os = "linux") {
        h.home_path().join(".config").join("agent-of-empires")
    } else {
        h.home_path().join(".agent-of-empires")
    };
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
    std::fs::write(config_dir.join("config.toml"), config_content).expect("write config.toml");
}

fn capture_target(h: &TuiTestHarness, target: &str) -> String {
    let output = Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args(["capture-pane", "-J", "-t", target, "-p"])
        .output()
        .expect("capture target pane");
    assert!(
        output.status.success(),
        "capture-pane failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn read_sessions_json(h: &TuiTestHarness) -> Option<serde_json::Value> {
    let sessions_path = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    };
    let content = std::fs::read_to_string(&sessions_path).ok()?;
    Some(serde_json::from_str(&content).expect("invalid sessions JSON"))
}

fn wait_for_tmux_session_name(h: &TuiTestHarness, title: &str) -> String {
    let start = std::time::Instant::now();
    while start.elapsed() <= Duration::from_secs(10) {
        if let Some(id) = read_sessions_json(h).and_then(|sessions| {
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

fn wait_for_target_contains(h: &TuiTestHarness, target: &str, expected: &str) {
    let start = std::time::Instant::now();
    while start.elapsed() <= Duration::from_secs(10) {
        let screen = capture_target(h, target);
        if screen.contains(expected) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    panic!(
        "Timed out waiting for {:?} in target {}.\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
        expected,
        target,
        capture_target(h, target)
    );
}

fn right_pane_line(screen: &str) -> &str {
    screen
        .lines()
        .find(|line| line.contains("Right Pane:"))
        .unwrap_or("")
}

#[test]
#[serial]
fn test_split_binding_uses_session_project_path() {
    crate::harness::require_tmux!();

    let h = TuiTestHarness::new("pane_cwd_inherit");
    let project = h.project_path();
    let expected_project = project
        .canonicalize()
        .expect("canonicalize project path")
        .display()
        .to_string();

    let add_output = h.run_cli(&[
        "add",
        project.to_str().expect("project path utf-8"),
        "-t",
        "Pane Cwd",
    ]);
    assert!(
        add_output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let start_output = h.run_cli_in_tmux(&["session", "start", "Pane Cwd"]);
    assert!(
        start_output.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start_output.stderr)
    );

    let fake_tmux_env = format!("{},999999,0", h.tmux_socket_path().display());
    let mut attach = h.spawn_cli_attach_process("Pane Cwd", Some(&fake_tmux_env));
    h.wait_for_client_count(1);

    let client_name = h.tmux_single_client_name();
    let session_name = h.tmux_client_session(&client_name);

    assert_eq!(
        h.tmux_show_option(&session_name, "@aoe_project_path"),
        expected_project.as_str()
    );

    h.send_keys_to_client(&client_name, "C-b");
    h.send_keys_to_client(&client_name, "%");

    let start = std::time::Instant::now();
    while start.elapsed() <= Duration::from_secs(10) {
        if h.tmux_display_message(&session_name, "#{window_panes}") == "2" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(
        h.tmux_display_message(&session_name, "#{window_panes}"),
        "2"
    );
    assert_eq!(
        h.tmux_display_message(&session_name, "#{pane_current_path}"),
        expected_project.as_str()
    );

    let _ = attach.kill();
    let _ = attach.wait();
}

#[test]
#[serial]
fn test_new_session_shell_right_pane_starts_in_project_path() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("right_pane_shell_cwd");
    write_shell_default_config(&h);

    let project = h.home_path().join("right-pane-project");
    std::fs::create_dir_all(&project).expect("create right pane project");
    let expected_project = project
        .canonicalize()
        .expect("canonicalize project path")
        .display()
        .to_string();

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.send_keys("n");
    h.wait_for("Title");

    h.type_text("Right Pane Shell");
    h.send_keys("Tab");
    for _ in 0..128 {
        h.send_keys("BSpace");
    }
    h.type_text(&expected_project);

    let tabs_to_right_pane = if h.capture_screen().contains("Tool:") {
        2
    } else {
        1
    };
    for _ in 0..tabs_to_right_pane {
        h.send_keys("Tab");
    }

    for _ in 0..32 {
        if right_pane_line(&h.capture_screen()).contains("● shell") {
            break;
        }
        h.send_keys("Right");
    }
    assert!(
        right_pane_line(&h.capture_screen()).contains("● shell"),
        "right pane tool should be set to shell\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
        h.capture_screen()
    );

    h.send_keys("Enter");

    let session_name = wait_for_tmux_session_name(&h, "Right Pane Shell");
    let start = std::time::Instant::now();
    while start.elapsed() <= Duration::from_secs(10) {
        if h.tmux_display_message(&session_name, "#{window_panes}") == "2" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(
        h.tmux_display_message(&session_name, "#{window_panes}"),
        "2",
        "expected the session to have a right pane"
    );

    let right_target = format!("{}:.1", session_name);
    assert_eq!(
        h.tmux_display_message(&right_target, "#{pane_current_path}"),
        expected_project
    );

    h.type_text_to_target(&right_target, "pwd");
    h.send_keys_to_target(&right_target, "Enter");
    wait_for_target_contains(&h, &right_target, &expected_project);
}
