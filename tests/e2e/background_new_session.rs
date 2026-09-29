//! e2e coverage for `n` creating a session without blocking the home list.
//!
//! A Cross Agent Team claude session waits up to 12s for the agent's startup
//! prompt while it launches, and the stub never renders one. If that launch
//! ran on the event loop the sort-order probe sent right after Enter would sit
//! unprocessed for the whole window.

use std::time::Duration;

use serial_test::serial;

use crate::harness::TuiTestHarness;

fn write_config(h: &TuiTestHarness) {
    let dir = if cfg!(target_os = "linux") {
        h.home_path().join(".config").join("agent-of-empires")
    } else {
        h.home_path().join(".agent-of-empires")
    };
    std::fs::create_dir_all(&dir).expect("create config dir");
    let config = format!(
        r#"[updates]
check_enabled = false

[app_state]
has_seen_welcome = true
last_seen_version = "{}"

[session]
default_tool = "claude"
cross_agent_team_default = true
"#,
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(dir.join("config.toml"), config).expect("write config.toml");
}

#[test]
#[serial]
fn n_returns_to_the_list_while_the_new_session_launches() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("bg_new_session");
    write_config(&h);
    h.install_tool_stub("claude");
    let project = h.project_path();

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.assert_screen_contains("Sort: Newest");

    h.send_keys("n");
    h.wait_for("New Session");
    h.type_text("Bg New");
    for _ in 0..3 {
        h.send_keys("Tab");
    }
    for _ in 0..128 {
        h.send_keys("BSpace");
    }
    h.type_text(project.to_str().unwrap());
    h.send_keys("Enter");
    h.send_keys("o");

    h.wait_for_timeout("Sort: Oldest", Duration::from_secs(4));
    h.wait_for("Bg New");
    h.wait_for("Starting...");
    h.assert_screen_not_contains("Restarting...");
    h.wait_for_absent("Starting...", Duration::from_secs(30));
    h.assert_screen_contains("Agent of Empires");
}
