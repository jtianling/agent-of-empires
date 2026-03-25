use serial_test::serial;
use std::process::Command;
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

fn client_context_option_key(prefix: &str, client_name: &str) -> String {
    let suffix: String = client_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{prefix}{suffix}")
}

fn add_and_start_cycle_sessions(h: &TuiTestHarness) -> (String, String, String) {
    let project = h.project_path();

    for args in [
        vec![
            "add",
            project.to_str().unwrap(),
            "-t",
            "Skills Manager Claude",
            "--group",
            "skills-manager",
            "--cmd-override",
            "sh",
        ],
        vec![
            "add",
            project.to_str().unwrap(),
            "-t",
            "Skills Manager Shell",
            "--group",
            "skills-manager",
            "--cmd-override",
            "sh",
        ],
        vec![
            "add",
            project.to_str().unwrap(),
            "-t",
            "Blog Writer",
            "--group",
            "blog-workspace",
            "--cmd-override",
            "sh",
        ],
    ] {
        let output = h.run_cli(&args);
        assert!(
            output.status.success(),
            "aoe add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    for title in [
        "Skills Manager Claude",
        "Skills Manager Shell",
        "Blog Writer",
    ] {
        let output = h.run_cli_in_tmux(&["session", "start", title]);
        assert!(
            output.status.success(),
            "aoe session start failed for {}: {}",
            title,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let sessions = read_sessions_json(h);
    let lookup_tmux = |title: &str| {
        let session = sessions
            .as_array()
            .expect("sessions array")
            .iter()
            .find(|session| session["title"].as_str() == Some(title))
            .unwrap_or_else(|| panic!("missing session {}", title));
        let id = session["id"].as_str().expect("session id");
        agent_of_empires::tmux::Session::generate_name(id, title)
    };

    (
        lookup_tmux("Skills Manager Claude"),
        lookup_tmux("Skills Manager Shell"),
        lookup_tmux("Blog Writer"),
    )
}

fn wait_for_file_size(path: &std::path::Path, expected_len: u64) {
    let start = std::time::Instant::now();
    while start.elapsed() <= Duration::from_secs(5) {
        if std::fs::metadata(path)
            .map(|metadata| metadata.len() >= expected_len)
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    panic!(
        "timed out waiting for {} to reach {} bytes",
        path.display(),
        expected_len
    );
}

fn tmux_show_option_optional(h: &TuiTestHarness, target: &str, option: &str) -> Option<String> {
    let output = Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args(["show-options", "-t", target, "-v", option])
        .output()
        .expect("failed to show tmux option");

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn tmux_show_global_option_optional(h: &TuiTestHarness, option: &str) -> Option<String> {
    let output = Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args(["show-options", "-g", "-v", option])
        .output()
        .expect("failed to show global tmux option");

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
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

#[test]
#[serial]
fn test_notification_monitor_tracks_hook_status_changes() {
    crate::harness::require_tmux!();

    let h = TuiTestHarness::new("cli_notification_monitor");
    let project = h.project_path();

    for title in ["Alpha Waiter", "Beta Waiter"] {
        let add_output = h.run_cli(&[
            "add",
            project.to_str().unwrap(),
            "-t",
            title,
            "--cmd-override",
            "sh",
        ]);
        assert!(
            add_output.status.success(),
            "aoe add failed for {}: {}",
            title,
            String::from_utf8_lossy(&add_output.stderr)
        );

        let start_output = h.run_cli_in_tmux(&["session", "start", title]);
        assert!(
            start_output.status.success(),
            "aoe session start failed for {}: {}",
            title,
            String::from_utf8_lossy(&start_output.stderr)
        );
    }

    let sessions = read_sessions_json(&h);
    let alpha = sessions
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|session| session["title"].as_str() == Some("Alpha Waiter"))
        .expect("alpha session");
    let beta = sessions
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|session| session["title"].as_str() == Some("Beta Waiter"))
        .expect("beta session");

    let alpha_id = alpha["id"].as_str().expect("alpha id");
    let beta_id = beta["id"].as_str().expect("beta id");
    let alpha_name = agent_of_empires::tmux::Session::generate_name(alpha_id, "Alpha Waiter");
    let beta_name = agent_of_empires::tmux::Session::generate_name(beta_id, "Beta Waiter");

    let monitor_deadline = std::time::Instant::now();
    while monitor_deadline.elapsed() <= Duration::from_secs(5) {
        if tmux_show_global_option_optional(&h, "@aoe_notification_monitor_pid").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let beta_hook_dir = agent_of_empires::hooks::hook_status_dir(beta_id);
    std::fs::create_dir_all(&beta_hook_dir).expect("create hook dir");
    std::fs::write(beta_hook_dir.join("status"), "waiting").expect("write waiting hook status");

    let start = std::time::Instant::now();
    let mut waiting_text = None;
    while start.elapsed() <= Duration::from_secs(8) {
        waiting_text = tmux_show_option_optional(&h, &alpha_name, "@aoe_waiting");
        if waiting_text
            .as_deref()
            .is_some_and(|text| text.contains("Beta Waiter"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(
        waiting_text
            .as_deref()
            .is_some_and(|text| text.contains("[1] ◐ Beta Waiter")),
        "expected waiting notification for Beta Waiter, got {:?}",
        waiting_text
    );
    assert_eq!(
        tmux_show_option_optional(&h, &alpha_name, "@aoe_notification_hint").as_deref(),
        None
    );

    std::fs::write(beta_hook_dir.join("status"), "running").expect("write running hook status");

    let start = std::time::Instant::now();
    let mut running_text = None;
    while start.elapsed() <= Duration::from_secs(8) {
        running_text = tmux_show_option_optional(&h, &alpha_name, "@aoe_waiting");
        if running_text
            .as_deref()
            .is_some_and(|text| text.contains("\u{25cf}"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(
        running_text
            .as_deref()
            .is_some_and(|text| text.contains("[1] \u{25cf} Beta Waiter")),
        "expected running notification for Beta Waiter, got {:?}",
        running_text
    );

    agent_of_empires::hooks::cleanup_hook_status_dir(beta_id);
    h.kill_tmux_target(&alpha_name);
    h.kill_tmux_target(&beta_name);
}

#[test]
#[serial]
fn test_hidden_switch_session_cycles_globally_and_preserves_return_target() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("cli_group_scoped_switch");
    let (skills_claude, skills_shell, blog_writer) = add_and_start_cycle_sessions(&h);

    h.spawn_tui();
    h.attach_control_client();
    h.wait_for("Skills Manager Claude");

    let client_name = h.tmux_single_client_name();
    let return_key = client_context_option_key("@aoe_return_session_", &client_name);
    h.tmux_set_global_option(&return_key, h.session_name());

    h.tmux_switch_client(&client_name, &skills_claude);
    h.wait_for_client_session(&client_name, &skills_claude);

    let switch_next = h.run_cli_in_tmux(&[
        "tmux",
        "switch-session",
        "--direction",
        "next",
        "--profile",
        "default",
        "--client-name",
        &client_name,
    ]);
    assert!(
        switch_next.status.success(),
        "aoe tmux switch-session failed: {}",
        String::from_utf8_lossy(&switch_next.stderr)
    );
    h.wait_for_client_session(&client_name, &blog_writer);
    assert_eq!(h.tmux_show_global_option(&return_key), h.session_name());

    let switch_wrap = h.run_cli_in_tmux(&[
        "tmux",
        "switch-session",
        "--direction",
        "next",
        "--profile",
        "default",
        "--client-name",
        &client_name,
    ]);
    assert!(
        switch_wrap.status.success(),
        "aoe tmux switch-session wrap failed: {}",
        String::from_utf8_lossy(&switch_wrap.stderr)
    );
    h.wait_for_client_session(&client_name, &skills_shell);
    assert_eq!(h.tmux_show_global_option(&return_key), h.session_name());

    let switch_global = h.run_cli_in_tmux(&[
        "tmux",
        "switch-session",
        "--direction",
        "next",
        "--profile",
        "default",
        "--client-name",
        &client_name,
    ]);
    assert!(
        switch_global.status.success(),
        "aoe tmux switch-session global cycle failed: {}",
        String::from_utf8_lossy(&switch_global.stderr)
    );
    h.wait_for_client_session(&client_name, &skills_claude);
    assert_eq!(h.tmux_show_global_option(&return_key), h.session_name());

    let rejected_global = h.run_cli_in_tmux(&[
        "tmux",
        "switch-session",
        "--direction",
        "next",
        "--global",
        "--profile",
        "default",
        "--client-name",
        &client_name,
    ]);
    assert!(
        !rejected_global.status.success(),
        "--global should be rejected once global cycling is the only mode"
    );
    let rejected_stderr = String::from_utf8_lossy(&rejected_global.stderr);
    assert!(
        rejected_stderr.contains("--global"),
        "expected clap error to mention --global, got: {}",
        rejected_stderr
    );

    let detach_cmd = concat!(
        "client_name=\"$(tmux display-message -p '#{client_name}')\"; ",
        "client_key=$(printf '%s' \"$client_name\" | tr -c '[:alnum:]' '_'); ",
        "tmux set-option -gq \"@aoe_last_detached_session_${client_key}\" \"$(tmux display-message -p '#{session_name}')\"; ",
        "target=$(tmux show-option -gv \"@aoe_return_session_${client_key}\" 2>/dev/null); ",
        "if [ -n \"$target\" ]; then ",
        "tmux switch-client -c \"$client_name\" -t \"$target\" 2>/dev/null || tmux detach-client -t \"$client_name\"; ",
        "else ",
        "tmux switch-client -c \"$client_name\" -l 2>/dev/null || tmux detach-client -t \"$client_name\"; ",
        "fi"
    );
    h.type_text_to_target(&skills_claude, detach_cmd);
    h.send_keys_to_target(&skills_claude, "Enter");
    h.wait_for_client_session(&client_name, h.session_name());
}

#[test]
#[serial]
fn test_root_cycle_bindings_work_after_attach_session_fallback() {
    crate::harness::require_tmux!();

    let h = TuiTestHarness::new("cli_root_cycle_attach_fallback");
    let (skills_claude, skills_shell, blog_writer) = add_and_start_cycle_sessions(&h);
    let fake_tmux_env = format!("{},999999,0", h.tmux_socket_path().display());

    let mut attach = h.spawn_cli_attach_process("Skills Manager Claude", Some(&fake_tmux_env));
    h.wait_for_client_count(1);

    let client_name = h.tmux_single_client_name();
    h.wait_for_client_session(&client_name, &skills_claude);

    h.send_keys_to_client(&client_name, "C-.");
    h.wait_for_client_session(&client_name, &blog_writer);

    h.send_keys_to_client(&client_name, "C-b");
    h.send_keys_to_client(&client_name, "b");
    h.wait_for_client_session(&client_name, &skills_claude);

    h.send_keys_to_client(&client_name, "C-,");
    h.wait_for_client_session(&client_name, &skills_shell);

    h.send_keys_to_client(&client_name, "C-b");
    h.send_keys_to_client(&client_name, "b");
    h.wait_for_client_session(&client_name, &skills_claude);

    let passthrough_session = "plain_passthrough";
    let passthrough_session_comma = "plain_passthrough_comma";
    let ctrl_dot_file = h.home_path().join("ctrl-dot.bin");
    let ctrl_comma_file = h.home_path().join("ctrl-comma.bin");
    h.create_detached_shell_session(passthrough_session);
    h.tmux_switch_client(&client_name, passthrough_session);
    h.wait_for_client_session(&client_name, passthrough_session);
    h.type_text_to_target(
        passthrough_session,
        &format!(
            "stty raw -echo; dd bs=1 count=1 of={} 2>/dev/null; stty sane",
            ctrl_dot_file.display()
        ),
    );
    h.send_keys_to_target(passthrough_session, "Enter");
    h.send_keys_to_client(&client_name, "C-.");
    wait_for_file_size(&ctrl_dot_file, 1);
    assert_eq!(
        std::fs::metadata(&ctrl_dot_file)
            .expect("ctrl-dot passthrough file metadata")
            .len(),
        1
    );

    h.create_detached_shell_session(passthrough_session_comma);
    h.tmux_switch_client(&client_name, passthrough_session_comma);
    h.wait_for_client_session(&client_name, passthrough_session_comma);
    h.type_text_to_target(
        passthrough_session_comma,
        &format!(
            "stty raw -echo; dd bs=1 count=1 of={} 2>/dev/null; stty sane",
            ctrl_comma_file.display()
        ),
    );
    h.send_keys_to_target(passthrough_session_comma, "Enter");
    h.send_keys_to_client(&client_name, "C-,");
    wait_for_file_size(&ctrl_comma_file, 1);
    assert_eq!(
        std::fs::metadata(&ctrl_comma_file)
            .expect("ctrl-comma passthrough file metadata")
            .len(),
        1
    );
    assert_eq!(
        h.tmux_client_session(&client_name),
        passthrough_session_comma
    );

    let _ = attach.kill();
    let _ = attach.wait();
}
