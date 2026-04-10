//! E2E tests for the fork session feature.

use serial_test::serial;
use std::time::Duration;

use crate::harness::{require_tmux, TuiTestHarness};

/// Helper to read the raw sessions.json from the harness's isolated home.
fn sessions_path(h: &TuiTestHarness) -> std::path::PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/sessions.json")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/sessions.json")
    }
}

fn read_sessions(h: &TuiTestHarness) -> serde_json::Value {
    let path = sessions_path(h);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
    serde_json::from_str(&content).expect("invalid sessions JSON")
}

/// Inject a `resume_token` into the first session in sessions.json whose title
/// matches `title`.
fn inject_resume_token(h: &TuiTestHarness, title: &str, token: &str) {
    let path = sessions_path(h);
    let content = std::fs::read_to_string(&path).expect("read sessions");
    let mut sessions: Vec<serde_json::Value> =
        serde_json::from_str(&content).expect("parse sessions");
    for s in &mut sessions {
        if s["title"].as_str() == Some(title) {
            s["resume_token"] = serde_json::Value::String(token.to_string());
        }
    }
    let patched = serde_json::to_string_pretty(&sessions).expect("serialize");
    std::fs::write(&path, patched).expect("write patched sessions");
}

// ---- CLI tests ----------------------------------------------------------

#[test]
#[serial]
fn test_cli_fork_creates_session() {
    require_tmux!();

    let h = TuiTestHarness::new("fork_cli_create");
    let project = h.project_path();

    // Create a parent session via CLI.
    let output = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "parent-cli",
        "-c",
        "claude",
    ]);
    assert!(
        output.status.success(),
        "aoe add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Inject a resume token so fork_token() succeeds.
    inject_resume_token(&h, "parent-cli", "fake-uuid-1234");

    // Fork without launching (we have no real claude binary to run).
    let output = h.run_cli(&["session", "fork", "parent-cli", "--no-launch"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "aoe session fork failed.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("Forked session"),
        "expected success message in stdout, got: {stdout}"
    );

    // Verify sessions.json now has two sessions.
    let sessions = read_sessions(&h);
    let arr = sessions.as_array().expect("sessions array");
    assert_eq!(arr.len(), 2, "expected 2 sessions (parent + fork)");

    let fork = arr
        .iter()
        .find(|s| s["title"].as_str() != Some("parent-cli"))
        .expect("fork session should exist");
    assert_eq!(
        fork["title"].as_str().unwrap_or(""),
        "parent-cli-fork",
        "default fork title"
    );
    assert_eq!(fork["tool"].as_str().unwrap_or(""), "claude");
    assert!(
        fork["parent_session_id"].as_str().is_some(),
        "fork should have parent_session_id"
    );
    assert_eq!(
        fork["fork_pending"].as_str().unwrap_or(""),
        "fake-uuid-1234",
        "fork_pending should store the parent token"
    );
    // On macOS, canonicalize may prepend /private. Normalize both sides.
    let fork_path = fork["project_path"].as_str().unwrap_or("");
    let expected = project.to_str().unwrap();
    assert!(
        fork_path.ends_with(expected.trim_start_matches("/private"))
            || expected.ends_with(fork_path.trim_start_matches("/private")),
        "fork shares parent's project_path: fork={fork_path}, expected={expected}"
    );
}

#[test]
#[serial]
fn test_cli_fork_custom_title_and_group() {
    require_tmux!();

    let h = TuiTestHarness::new("fork_cli_custom");
    let project = h.project_path();

    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "base",
        "-c",
        "claude",
    ]);
    inject_resume_token(&h, "base", "tok-aaa");

    let output = h.run_cli(&[
        "session",
        "fork",
        "base",
        "--title",
        "experiment-1",
        "--group",
        "experiments",
        "--no-launch",
    ]);
    assert!(output.status.success());

    let sessions = read_sessions(&h);
    let fork = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str() == Some("experiment-1"))
        .expect("fork with custom title");
    assert_eq!(fork["group_path"].as_str().unwrap_or(""), "experiments");
}

#[test]
#[serial]
fn test_cli_fork_rejects_unsupported_tool() {
    require_tmux!();

    let h = TuiTestHarness::new("fork_cli_unsupported");
    let project = h.project_path();

    // Create a session with gemini tool (not forkable).
    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "gemini-session",
        "-c",
        "gemini",
    ]);
    inject_resume_token(&h, "gemini-session", "ignored");

    let output = h.run_cli(&["session", "fork", "gemini-session", "--no-launch"]);
    assert!(
        !output.status.success(),
        "fork should fail for unsupported tool"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not supported") || stderr.contains("Not supported"),
        "expected unsupported error, got: {stderr}"
    );
}

#[test]
#[serial]
fn test_cli_fork_rejects_no_token_codex() {
    require_tmux!();

    let h = TuiTestHarness::new("fork_cli_no_token");
    let project = h.project_path();

    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "no-token",
        "-c",
        "codex",
    ]);
    // Do NOT inject resume_token.

    let output = h.run_cli(&["session", "fork", "no-token", "--no-launch"]);
    assert!(
        !output.status.success(),
        "fork should fail without resume token"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No active codex session"),
        "expected missing-token error, got: {stderr}"
    );
}

#[test]
#[serial]
fn test_cli_fork_codex_session() {
    require_tmux!();

    let h = TuiTestHarness::new("fork_cli_codex");
    let project = h.project_path();

    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "codex-parent",
        "-c",
        "codex",
    ]);
    inject_resume_token(&h, "codex-parent", "codex-uuid-999");

    let output = h.run_cli(&["session", "fork", "codex-parent", "--no-launch"]);
    assert!(
        output.status.success(),
        "codex fork should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let sessions = read_sessions(&h);
    let fork = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str() == Some("codex-parent-fork"))
        .expect("codex fork");
    assert_eq!(fork["tool"].as_str().unwrap_or(""), "codex");
    assert_eq!(
        fork["fork_pending"].as_str().unwrap_or(""),
        "codex-uuid-999"
    );
}

// ---- TUI tests ----------------------------------------------------------

#[test]
#[serial]
fn test_tui_fork_dialog_opens_and_submits() {
    require_tmux!();

    let mut h = TuiTestHarness::new("fork_tui_dialog");
    let project = h.project_path();

    // Create parent session via CLI first.
    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "tui-parent",
        "-c",
        "claude",
    ]);
    inject_resume_token(&h, "tui-parent", "tui-fake-uuid");

    // Start the TUI.
    h.spawn_tui();
    h.wait_for("tui-parent");

    // Press f to open fork dialog.
    h.send_keys("f");
    h.wait_for("Fork Session");
    h.assert_screen_contains("Parent:");
    h.assert_screen_contains("tui-parent");
    h.assert_screen_contains("tui-parent-fork");

    // Submit with defaults.
    h.send_keys("Enter");

    // After submit, the TUI tries to attach to the fork. Even if the claude
    // stub exits immediately, the fork should be persisted. Give it a moment
    // then check sessions.json.
    std::thread::sleep(Duration::from_secs(2));

    let sessions = read_sessions(&h);
    let arr = sessions.as_array().expect("sessions array");
    assert!(
        arr.len() >= 2,
        "expected at least 2 sessions, got {}",
        arr.len()
    );
    assert!(
        arr.iter()
            .any(|s| s["title"].as_str() == Some("tui-parent-fork")),
        "fork should be persisted in sessions.json: {sessions}"
    );
}

#[test]
#[serial]
fn test_tui_fork_shows_not_ready_without_token_codex() {
    require_tmux!();

    let mut h = TuiTestHarness::new("fork_tui_no_token");
    let project = h.project_path();

    // Create a fake codex stub
    let stub_dir = h.home_path().join("fake-bin-codex");
    std::fs::create_dir_all(&stub_dir).unwrap();
    let codex_stub = stub_dir.join("codex");
    std::fs::write(&codex_stub, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&codex_stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "no-tok",
        "-c",
        "codex",
    ]);
    // No resume_token injected.

    h.spawn_tui();
    h.wait_for("no-tok");

    h.send_keys("f");
    h.wait_for("Fork Not Ready");
    h.assert_screen_contains("restart");

    // Esc dismisses the info dialog.
    h.send_keys("Escape");
    h.wait_for_absent("Fork Not Ready", Duration::from_secs(3));
}

#[test]
#[serial]
fn test_tui_fork_shows_not_supported_for_gemini() {
    require_tmux!();

    let mut h = TuiTestHarness::new("fork_tui_unsupported");
    let project = h.project_path();

    // Create a fake gemini stub
    let stub_dir = h.home_path().join("fake-bin");
    std::fs::create_dir_all(&stub_dir).unwrap();
    let gemini_stub = stub_dir.join("gemini");
    std::fs::write(&gemini_stub, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gemini_stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "gem-session",
        "-c",
        "gemini",
    ]);

    h.spawn_tui();
    h.wait_for("gem-session");

    h.send_keys("f");
    h.wait_for("Fork Not Supported");
    // Text wraps in the info dialog; check a substring that fits one line.
    h.assert_screen_contains("claude, codex, and");

    h.send_keys("Escape");
    h.wait_for_absent("Fork Not Supported", Duration::from_secs(3));
}

#[test]
#[serial]
fn test_tui_fork_dialog_cancel() {
    require_tmux!();

    let mut h = TuiTestHarness::new("fork_tui_cancel");
    let project = h.project_path();

    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "cancel-parent",
        "-c",
        "claude",
    ]);
    inject_resume_token(&h, "cancel-parent", "cancel-uuid");

    h.spawn_tui();
    h.wait_for("cancel-parent");

    h.send_keys("f");
    h.wait_for("Fork Session");

    // Press Esc to cancel.
    h.send_keys("Escape");
    h.wait_for_absent("Fork Session", Duration::from_secs(3));

    // Only the parent should remain.
    let sessions = read_sessions(&h);
    let arr = sessions.as_array().expect("sessions array");
    assert_eq!(arr.len(), 1, "cancel should not create a fork");
}

// ---- Pre-allocated session ID tests -------------------------------------

/// When a Claude session is started, AoE should pre-allocate a UUID and
/// persist it as `agent_session_id`.
#[test]
#[serial]
fn test_claude_start_preallocates_session_id() {
    require_tmux!();

    let h = TuiTestHarness::new("fork_prealloc");
    let project = h.project_path();

    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "prealloc",
        "-c",
        "claude",
        "--launch",
    ]);
    std::thread::sleep(Duration::from_secs(1));

    let sessions = read_sessions(&h);
    let parent = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str() == Some("prealloc"))
        .expect("parent session");

    let session_id = parent["agent_session_id"]
        .as_str()
        .expect("agent_session_id should be set after start");

    // UUID format: 36 chars with hyphens (8-4-4-4-12).
    assert_eq!(session_id.len(), 36, "UUID length");
    assert!(
        session_id.contains('-'),
        "UUID should contain hyphens: {session_id}"
    );
}

/// Fork of a started parent should use parent's `agent_session_id` as the
/// fork token (no resume_token injection needed).
#[test]
#[serial]
fn test_fork_uses_preallocated_session_id() {
    require_tmux!();

    let h = TuiTestHarness::new("fork_use_prealloc");
    let project = h.project_path();

    // Create and start parent so agent_session_id is set.
    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "started-parent",
        "-c",
        "claude",
        "--launch",
    ]);
    std::thread::sleep(Duration::from_secs(1));

    let sessions = read_sessions(&h);
    let parent = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str() == Some("started-parent"))
        .expect("parent");
    let parent_agent_id = parent["agent_session_id"]
        .as_str()
        .expect("parent should have agent_session_id");

    // Fork without launching.
    let output = h.run_cli(&["session", "fork", "started-parent", "--no-launch"]);
    assert!(
        output.status.success(),
        "fork should succeed with agent_session_id: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let sessions = read_sessions(&h);
    let fork = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str() == Some("started-parent-fork"))
        .expect("fork");

    // fork_pending should be the parent's pre-allocated ID.
    assert_eq!(
        fork["fork_pending"].as_str().unwrap_or(""),
        parent_agent_id,
        "fork_pending should use parent's agent_session_id"
    );

    // The fork should also have its OWN pre-allocated agent_session_id.
    let fork_agent_id = fork["agent_session_id"]
        .as_str()
        .expect("fork should have its own agent_session_id");
    assert_ne!(
        fork_agent_id, parent_agent_id,
        "fork's agent_session_id must differ from parent's"
    );
    assert_eq!(fork_agent_id.len(), 36, "fork UUID length");
}

/// Fork of fork: verify the chain works (fork's agent_session_id becomes
/// grandchild's fork_pending).
#[test]
#[serial]
fn test_fork_of_fork() {
    require_tmux!();

    let h = TuiTestHarness::new("fork_chain");
    let project = h.project_path();

    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "gen0",
        "-c",
        "claude",
        "--launch",
    ]);
    std::thread::sleep(Duration::from_secs(1));

    // Fork gen0 -> gen1
    let output = h.run_cli(&["session", "fork", "gen0", "-t", "gen1", "--no-launch"]);
    assert!(output.status.success(), "gen0->gen1 fork failed");

    // Fork gen1 -> gen2 (fork of fork)
    let output = h.run_cli(&["session", "fork", "gen1", "-t", "gen2", "--no-launch"]);
    assert!(
        output.status.success(),
        "gen1->gen2 fork failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let sessions = read_sessions(&h);
    let arr = sessions.as_array().unwrap();
    assert_eq!(arr.len(), 3, "should have gen0, gen1, gen2");

    let gen1 = arr
        .iter()
        .find(|s| s["title"].as_str() == Some("gen1"))
        .unwrap();
    let gen2 = arr
        .iter()
        .find(|s| s["title"].as_str() == Some("gen2"))
        .unwrap();

    // gen2's fork_pending should be gen1's agent_session_id.
    let gen1_agent_id = gen1["agent_session_id"].as_str().unwrap();
    assert_eq!(
        gen2["fork_pending"].as_str().unwrap_or(""),
        gen1_agent_id,
        "grandchild's fork_pending should be child's agent_session_id"
    );

    // gen2's parent_session_id should point to gen1 (not gen0).
    let gen1_id = gen1["id"].as_str().unwrap();
    assert_eq!(
        gen2["parent_session_id"].as_str().unwrap_or(""),
        gen1_id,
        "gen2 should be a child of gen1"
    );
}

/// After a forked session is started, fork_pending should be cleared and
/// agent_session_id should remain.
#[test]
#[serial]
fn test_fork_pending_cleared_after_start() {
    require_tmux!();

    let h = TuiTestHarness::new("fork_clear_pending");
    let project = h.project_path();

    h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        "cleartest",
        "-c",
        "claude",
        "--launch",
    ]);
    std::thread::sleep(Duration::from_secs(1));

    // Fork AND launch the fork.
    let output = h.run_cli(&["session", "fork", "cleartest"]);
    assert!(
        output.status.success(),
        "fork+launch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::thread::sleep(Duration::from_secs(1));

    let sessions = read_sessions(&h);
    let fork = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str() == Some("cleartest-fork"))
        .expect("fork session");

    // fork_pending should be cleared after successful start.
    assert!(
        fork.get("fork_pending").is_none()
            || fork["fork_pending"].is_null()
            || fork["fork_pending"].as_str() == Some(""),
        "fork_pending should be cleared after start, got: {:?}",
        fork.get("fork_pending")
    );

    // agent_session_id should still be present.
    assert!(
        fork["agent_session_id"].as_str().is_some(),
        "agent_session_id should persist after start"
    );
}
