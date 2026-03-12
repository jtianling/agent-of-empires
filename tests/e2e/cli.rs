use serial_test::serial;
use std::time::Duration;

use crate::harness::TuiTestHarness;

/// Helper to read a session field from the sessions.json in the harness's isolated home.
fn read_sessions_json(h: &TuiTestHarness) -> serde_json::Value {
    let sessions_path = if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    };
    let content = std::fs::read_to_string(&sessions_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", sessions_path.display(), e));
    serde_json::from_str(&content).expect("invalid sessions JSON")
}

#[test]
#[serial]
fn test_cli_add_and_list() {
    let h = TuiTestHarness::new("cli_add_list");
    let project = h.project_path();

    let add_output = h.run_cli(&["add", project.to_str().unwrap(), "-t", "E2E Test Session"]);
    assert!(
        add_output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let list_output = h.run_cli(&["list"]);
    assert!(
        list_output.status.success(),
        "aoe list failed: {}",
        String::from_utf8_lossy(&list_output.stderr)
    );

    let stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        stdout.contains("E2E Test Session"),
        "list output should contain session title.\nOutput:\n{}",
        stdout
    );
}

#[test]
#[serial]
fn test_cli_add_invalid_path() {
    let h = TuiTestHarness::new("cli_add_invalid");

    let output = h.run_cli(&["add", "/nonexistent/path/that/does/not/exist"]);
    assert!(
        !output.status.success(),
        "aoe add should fail for nonexistent path"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("not")
            || combined.contains("exist")
            || combined.contains("error")
            || combined.contains("Error")
            || combined.contains("invalid")
            || combined.contains("No such"),
        "expected error message about invalid path.\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );
}

#[test]
#[serial]
fn test_cli_add_respects_config_extra_args() {
    let h = TuiTestHarness::new("cli_add_config_extra_args");
    let project = h.project_path();

    // Write config with agent_extra_args for claude
    let config_dir = if cfg!(target_os = "linux") {
        h.home_path().join(".config/agent-of-empires")
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
agent_extra_args = {{ claude = "--verbose --debug" }}
"#,
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(config_dir.join("config.toml"), config_content).expect("write config.toml");

    let add_output = h.run_cli(&["add", project.to_str().unwrap(), "-t", "ConfigExtraArgs"]);
    assert!(
        add_output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let sessions = read_sessions_json(&h);
    let session = &sessions[0];
    assert_eq!(
        session["extra_args"].as_str().unwrap_or(""),
        "--verbose --debug",
        "extra_args should be populated from config"
    );
}

#[test]
#[serial]
fn test_cli_add_respects_config_command_override() {
    let h = TuiTestHarness::new("cli_add_config_cmd_override");
    let project = h.project_path();

    // Write config with agent_command_override for claude
    let config_dir = if cfg!(target_os = "linux") {
        h.home_path().join(".config/agent-of-empires")
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
agent_command_override = {{ claude = "my-custom-claude" }}
"#,
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(config_dir.join("config.toml"), config_content).expect("write config.toml");

    let add_output = h.run_cli(&["add", project.to_str().unwrap(), "-t", "ConfigCmdOverride"]);
    assert!(
        add_output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let sessions = read_sessions_json(&h);
    let session = &sessions[0];
    assert_eq!(
        session["command"].as_str().unwrap_or(""),
        "my-custom-claude",
        "command should be populated from config agent_command_override"
    );
}

#[test]
#[serial]
fn test_cli_add_cli_flags_override_config() {
    let h = TuiTestHarness::new("cli_add_flags_override");
    let project = h.project_path();

    // Write config with agent_extra_args for claude
    let config_dir = if cfg!(target_os = "linux") {
        h.home_path().join(".config/agent-of-empires")
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
agent_extra_args = {{ claude = "--from-config" }}
agent_command_override = {{ claude = "config-claude" }}
"#,
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(config_dir.join("config.toml"), config_content).expect("write config.toml");

    // CLI flags should take priority over config
    let add_output = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "FlagsOverride",
        "--extra-args",
        "from-cli-extra",
        "--cmd-override",
        "cli-claude",
    ]);
    assert!(
        add_output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let sessions = read_sessions_json(&h);
    let session = &sessions[0];
    assert_eq!(
        session["extra_args"].as_str().unwrap_or(""),
        "from-cli-extra",
        "CLI --extra-args should override config"
    );
    assert_eq!(
        session["command"].as_str().unwrap_or(""),
        "cli-claude",
        "CLI --cmd-override should override config"
    );
}

#[test]
#[serial]
fn test_started_session_enables_tmux_title_passthrough() {
    let h = TuiTestHarness::new("cli_tmux_title_passthrough");
    let project = h.project_path();

    let add_output = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "Title Passthrough",
        "--cmd-override",
        "sh",
    ]);
    assert!(
        add_output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let start_output = h.run_cli_in_tmux(&["session", "start", "Title Passthrough"]);
    assert!(
        start_output.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start_output.stderr)
    );

    let sessions = read_sessions_json(&h);
    let session = &sessions[0];
    let session_id = session["id"].as_str().expect("session id");
    let session_title = session["title"].as_str().expect("session title");
    let tmux_name = agent_of_empires::tmux::Session::generate_name(session_id, session_title);

    assert_eq!(h.tmux_show_option(&tmux_name, "set-titles"), "on");
    assert_eq!(h.tmux_show_option(&tmux_name, "set-titles-string"), "#T");
    assert_eq!(
        h.tmux_show_window_option(&tmux_name, "allow-set-title"),
        "on"
    );
    assert_eq!(
        h.tmux_display_message(&tmux_name, "#{pane_title}"),
        "Title Passthrough"
    );

    h.type_text_to_target(&tmux_name, "printf '\\033]2;Runtime Title\\033\\\\'");
    h.send_keys_to_target(&tmux_name, "Enter");

    let mut runtime_title = String::new();
    for _ in 0..20 {
        runtime_title = h.tmux_display_message(&tmux_name, "#{pane_title}");
        if runtime_title == "Runtime Title" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(runtime_title, "Runtime Title");
    h.kill_tmux_target(&tmux_name);
}

#[test]
#[serial]
fn test_codex_session_waiting_title_uses_hand_icon() {
    crate::harness::require_tmux!();

    let h = TuiTestHarness::new("cli_codex_waiting_title");
    let project = h.project_path();

    let add_output = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "Codex Wait Title",
        "-c",
        "codex",
        "--cmd-override",
        "sh",
    ]);
    assert!(
        add_output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let start_output = h.run_cli_in_tmux(&["session", "start", "Codex Wait Title"]);
    assert!(
        start_output.status.success(),
        "aoe session start failed: {}",
        String::from_utf8_lossy(&start_output.stderr)
    );

    let sessions = read_sessions_json(&h);
    let session = &sessions[0];
    let session_id = session["id"].as_str().expect("session id");
    let session_title = session["title"].as_str().expect("session title");
    let tmux_name = agent_of_empires::tmux::Session::generate_name(session_id, session_title);

    let mut pane_title = String::new();
    for _ in 0..20 {
        pane_title = h.tmux_display_message(&tmux_name, "#{pane_title}");
        if pane_title == "Codex Wait Title" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(pane_title, "Codex Wait Title");

    h.type_text_to_target(
        &tmux_name,
        "printf '› Ask Codex to do anything\\n'; sleep 2",
    );
    h.send_keys_to_target(&tmux_name, "Enter");

    let mut waiting_title = String::new();
    for _ in 0..40 {
        waiting_title = h.tmux_display_message(&tmux_name, "#{pane_title}");
        if waiting_title == "✋ Codex Wait Title" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(waiting_title, "✋ Codex Wait Title");

    h.type_text_to_target(
        &tmux_name,
        "i=1; while [ $i -le 80 ]; do echo file-saved-$i; i=$((i+1)); done",
    );
    h.send_keys_to_target(&tmux_name, "Enter");

    let mut resumed_title = String::new();
    for _ in 0..40 {
        resumed_title = h.tmux_display_message(&tmux_name, "#{pane_title}");
        if resumed_title == "Codex Wait Title" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(resumed_title, "Codex Wait Title");

    h.kill_tmux_target(&tmux_name);
}
