//! Runtime coverage for the xats identity key AoE injects into Cross Agent Team
//! panes.
//!
//! End-to-end identity continuity cannot be asserted here: it needs the xats
//! daemon to accept the key at registration and resolve it back to a team and
//! name at reconnect, and that half is a separate project. Acceptance is
//! therefore scoped to what AoE controls: the key is present at a pane's first
//! launch, it is stable across restart and recovery, an extra pane's key is
//! fresh rather than the primary pane's, no key is injected when Cross Agent
//! Team is off, and slot 0 keeps a resume path before its first capture.

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
    start_cross_agent_team_session(h, title)
}

/// As above, for a test that has already installed the stub it needs.
fn start_cross_agent_team_session(h: &TuiTestHarness, title: &str) -> String {
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

/// Install a stub that records the environment the launched process actually
/// received, keyed by the pane it ran in.
///
/// The key is read back from that dump rather than from a shell inside the pane:
/// a pane's interactive shell carries its own environment, and the defect this
/// covers was an agent process running with no key at all while the pane looked
/// healthy from the outside.
/// Only the one variable under test is written out. Dumping the whole
/// environment would put whatever credentials the developer's shell carries into
/// a temp file that a failed run leaves behind.
fn install_env_dumping_stub(h: &TuiTestHarness, name: &str) {
    std::fs::create_dir_all(pane_env_dir(h)).expect("create pane env dir");
    h.install_stub_script(
        name,
        &format!(
            "#!/bin/sh\nprintf 'XATS_IDENTITY_KEY=%s\\n' \"${{XATS_IDENTITY_KEY-}}\" \
             > '{}'/\"${{TMUX_PANE#%}}\"\nexec sleep 2147483647\n",
            pane_env_dir(h).display()
        ),
    );
}

fn pane_env_dir(h: &TuiTestHarness) -> PathBuf {
    h.home_path().join("pane-env")
}

fn pane_env_file(h: &TuiTestHarness, pane_id: &str) -> PathBuf {
    pane_env_dir(h).join(pane_id.trim_start_matches('%'))
}

/// Wait for the launched process to dump its environment, then return the
/// identity key it carries (empty when it has none).
fn wait_for_pane_env_key(h: &TuiTestHarness, pane_id: &str) -> String {
    let path = pane_env_file(h, pane_id);
    let start = Instant::now();
    loop {
        if let Ok(env) = std::fs::read_to_string(&path) {
            let key = env
                .lines()
                .find_map(|line| line.strip_prefix("XATS_IDENTITY_KEY="))
                .unwrap_or("")
                .to_string();
            if !key.is_empty() {
                return key;
            }
            if start.elapsed() > Duration::from_secs(5) {
                return key;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "pane {pane_id} never dumped its environment to {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Wait for a session created through the TUI to reach the store, then return
/// its id.
fn wait_for_session_id(h: &TuiTestHarness, title: &str) -> String {
    let start = Instant::now();
    loop {
        if let Some(id) = std::fs::read_to_string(sessions_path(h))
            .ok()
            .and_then(|c| serde_json::from_str::<Vec<serde_json::Value>>(&c).ok())
            .and_then(|sessions| {
                sessions.iter().find_map(|s| {
                    (s["title"] == title).then(|| s["id"].as_str().unwrap_or("").to_string())
                })
            })
            .filter(|id| !id.is_empty())
        {
            return id;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "session {title} was never created\n\n--- Screen capture ---\n{}",
            h.capture_screen()
        );
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn pane_id_at(h: &TuiTestHarness, session_name: &str, index: usize) -> String {
    h.tmux_display_message(&format!("{session_name}.{index}"), "#{pane_id}")
}

fn wait_for_pane_count(h: &TuiTestHarness, session_name: &str, expected: &str) {
    let start = Instant::now();
    loop {
        let panes = h.tmux_display_message(session_name, "#{window_panes}");
        if panes == expected {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "expected {expected} panes, got {panes}"
        );
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn slot_column(h: &TuiTestHarness, id: &str, pane_id: &str, column: &str) -> String {
    sqlite_query(
        &db_path(h),
        &format!(
            "SELECT {column} FROM agent_slot \
             WHERE instance_id='{id}' AND tmux_pane='{pane_id}';"
        ),
    )
}

fn wait_for_slot_row(h: &TuiTestHarness, id: &str, pane_id: &str) {
    let start = Instant::now();
    loop {
        if !slot_column(h, id, pane_id, "tmux_pane").is_empty() {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "pane {pane_id} never got a durable slot record"
        );
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn right_pane_line(screen: &str) -> &str {
    screen
        .lines()
        .find(|line| line.contains("Right Pane Agent:"))
        .unwrap_or("")
}

/// Move from the right-pane selector to that pane's Cross Agent Team checkbox.
fn enable_cross_agent_team(h: &TuiTestHarness) {
    for _ in 0..3 {
        h.send_keys("Tab");
    }
    h.send_keys("Space");
    assert!(
        h.capture_screen().contains("Cross Agent Team: [x]"),
        "could not enable Cross Agent Team\n\n--- Screen capture ---\n{}",
        h.capture_screen(),
    );
}

/// The right pane of a new session is a pane AoE built the command for, so it
/// has no reason to reach xats keyless. A pane that holds no key is exactly what
/// the daemon's seat matching treats as claimable.
#[test]
#[serial]
fn new_session_right_pane_is_launched_with_its_own_key() {
    crate::harness::require_tmux!();
    if !sqlite3_available() {
        eprintln!("skipping: sqlite3 not available");
        return;
    }

    let mut h = TuiTestHarness::new("xats_identity_right_pane");
    install_env_dumping_stub(&h, "claude");
    let project = h.project_path().to_str().unwrap().to_string();

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.send_keys("n");
    h.wait_for("Title");

    h.type_text("Right Pane Key");
    for _ in 0..3 {
        h.send_keys("Tab");
    }
    for _ in 0..128 {
        h.send_keys("BSpace");
    }
    h.type_text(&project);

    for _ in 0..4 {
        h.send_keys("Tab");
    }
    for _ in 0..32 {
        if right_pane_line(&h.capture_screen()).contains("● claude") {
            break;
        }
        h.send_keys("Right");
    }
    assert!(
        right_pane_line(&h.capture_screen()).contains("● claude"),
        "right pane tool should be claude\n\n--- Screen capture ---\n{}",
        h.capture_screen()
    );

    enable_cross_agent_team(&h);
    h.send_keys("Enter");

    let id = wait_for_session_id(&h, "Right Pane Key");
    let session_name = agent_of_empires::tmux::Session::generate_name(&id, "Right Pane Key");
    wait_for_pane_count(&h, &session_name, "2");

    let left = pane_id_at(&h, &session_name, 0);
    let right = pane_id_at(&h, &session_name, 1);
    let right_key = wait_for_pane_env_key(&h, &right);
    let left_key = wait_for_pane_env_key(&h, &left);

    assert!(
        !right_key.is_empty(),
        "the right pane process must carry an identity key from its first launch"
    );
    assert!(
        left_key.is_empty(),
        "the disabled primary pane must stay keyless"
    );
    assert!(stored_identity_key(&h, &id).is_none());

    wait_for_slot_row(&h, &id, &right);
    assert_eq!(
        slot_column(&h, &id, &right, "xats_identity_key"),
        right_key,
        "the key in the pane's environment must be the one on its slot record"
    );
}

/// The extra pane must be restartable and keep its identity before it has been
/// captured: on Codex the claim lands only after the pane's first exchange, and
/// a restart in that window used to skip the pane entirely.
#[test]
#[serial]
fn an_added_pane_keeps_its_key_across_a_restart() {
    crate::harness::require_tmux!();
    if !sqlite3_available() {
        eprintln!("skipping: sqlite3 not available");
        return;
    }

    let mut h = TuiTestHarness::new("xats_identity_added_pane");
    install_env_dumping_stub(&h, "claude");
    let id = start_cross_agent_team_session(&h, "Added Pane Key");
    let session_name = agent_of_empires::tmux::Session::generate_name(&id, "Added Pane Key");

    let added = h.run_cli_in_tmux(&["session", "add-agent-pane", "Added Pane Key"]);
    assert!(
        added.status.success(),
        "aoe session add-agent-pane failed: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    wait_for_pane_count(&h, &session_name, "2");

    let left = pane_id_at(&h, &session_name, 0);
    let right = pane_id_at(&h, &session_name, 1);
    let before = wait_for_pane_env_key(&h, &right);
    let left_key = wait_for_pane_env_key(&h, &left);

    assert!(
        !before.is_empty(),
        "a pane added through the CLI must carry an identity key"
    );
    assert_ne!(
        before, left_key,
        "the added pane must not reuse the key AoE injected into the primary pane"
    );

    wait_for_slot_row(&h, &id, &right);
    assert_eq!(slot_column(&h, &id, &right, "xats_identity_key"), before);
    assert_eq!(
        slot_column(&h, &id, &right, "native_session_id"),
        "",
        "the record exists before the pane has reported a conversation"
    );
    // The fan-out restart reads only the slots that exist, so tracking the added
    // pane alone would take the primary pane out of it.
    wait_for_slot_row(&h, &id, &left);
    assert_eq!(
        slot_column(&h, &id, &left, "slot"),
        "0",
        "the primary pane is tracked beside the pane that was just added"
    );

    // Nothing has captured either pane, so this restart reaches the added pane
    // only because its slot record was written at launch.
    std::fs::remove_file(pane_env_file(&h, &right)).expect("clear the added pane's env dump");
    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.send_keys("C");

    let after = wait_for_pane_env_key(&h, &right);
    assert_eq!(
        after, before,
        "a relaunched pane must reuse the key it registered under, not mint a new one"
    );
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
