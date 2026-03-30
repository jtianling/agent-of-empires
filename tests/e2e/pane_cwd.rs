use std::time::Duration;

use serial_test::serial;

use crate::harness::TuiTestHarness;

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
