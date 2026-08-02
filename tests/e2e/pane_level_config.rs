//! End-to-end coverage for independent primary and secondary pane config.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serial_test::serial;

use crate::harness::TuiTestHarness;

#[derive(Debug)]
struct SlotConfig {
    slot: i64,
    agent: String,
    cwd: PathBuf,
    yolo_mode: bool,
    cross_agent_team: bool,
    identity_key: String,
    worktree: agent_of_empires::session::PaneWorktreeInfo,
}

fn config_dir(h: &TuiTestHarness) -> PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path().join(".config/agent-of-empires")
    } else {
        h.home_path().join(".agent-of-empires")
    }
}

fn write_test_config(h: &TuiTestHarness, worktree_root: &Path) {
    let content = format!(
        r#"[updates]
check_enabled = false

[app_state]
has_seen_welcome = true
last_seen_version = "{}"

[session]
default_tool = "claude"
yolo_mode_default = false
cross_agent_team_default = false

[worktree]
path_template = "{}/{{branch}}"
"#,
        env!("CARGO_PKG_VERSION"),
        worktree_root.display(),
    );
    std::fs::write(config_dir(h).join("config.toml"), content).expect("write config.toml");
}

fn create_repository(path: &Path) {
    std::fs::create_dir_all(path).expect("create repository directory");
    let init = Command::new("git")
        .args(["init", "-q"])
        .arg(path)
        .output()
        .expect("git init");
    assert!(init.status.success(), "git init failed");
    std::fs::write(path.join("README.md"), "pane config e2e\n").expect("write fixture");
    let add = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["add", "README.md"])
        .output()
        .expect("git add");
    assert!(add.status.success(), "git add failed");
    let commit = Command::new("git")
        .arg("-C")
        .arg(path)
        .args([
            "-c",
            "user.name=AoE E2E",
            "-c",
            "user.email=aoe-e2e@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ])
        .output()
        .expect("git commit");
    assert!(
        commit.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );
}

fn clear_focused_text(h: &TuiTestHarness) {
    for _ in 0..256 {
        h.send_keys("BSpace");
    }
}

fn right_pane_line(screen: &str) -> &str {
    screen
        .lines()
        .find(|line| line.contains("Right Pane Agent:"))
        .unwrap_or("")
}

fn select_codex_right_pane(h: &TuiTestHarness) {
    for _ in 0..32 {
        if right_pane_line(&h.capture_screen()).contains("● codex") {
            return;
        }
        h.send_keys("Right");
    }
    panic!(
        "codex was not selectable as the right pane agent\n{}",
        h.capture_screen()
    );
}

fn session_id(h: &TuiTestHarness, title: &str) -> Option<String> {
    let path = config_dir(h).join("profiles/default/sessions.json");
    let content = std::fs::read_to_string(path).ok()?;
    let sessions: serde_json::Value = serde_json::from_str(&content).ok()?;
    sessions.as_array()?.iter().find_map(|session| {
        (session["title"].as_str() == Some(title))
            .then(|| session["id"].as_str().map(str::to_string))
            .flatten()
    })
}

fn read_slots(h: &TuiTestHarness, instance_id: &str) -> anyhow::Result<Vec<SlotConfig>> {
    let conn = Connection::open(config_dir(h).join("profiles/default/aoe.db"))?;
    let mut statement = conn.prepare(
        "SELECT slot, agent, cwd, yolo_mode, cross_agent_team, xats_identity_key, \
                worktree_info \
         FROM agent_slot WHERE instance_id = ?1 ORDER BY slot",
    )?;
    let rows = statement.query_map([instance_id], |row| {
        let worktree_json: String = row.get(6)?;
        let worktree = serde_json::from_str(&worktree_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                worktree_json.len(),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(SlotConfig {
            slot: row.get(0)?,
            agent: row.get(1)?,
            cwd: PathBuf::from(row.get::<_, String>(2)?),
            yolo_mode: row.get::<_, i64>(3)? != 0,
            cross_agent_team: row.get::<_, i64>(4)? != 0,
            identity_key: row.get(5)?,
            worktree,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn wait_for_two_slots(h: &TuiTestHarness, instance_id: &str) -> Vec<SlotConfig> {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() <= Duration::from_secs(20) {
        match read_slots(h, instance_id) {
            Ok(slots) if slots.len() == 2 => return slots,
            Ok(slots) => last = format!("found {} slots", slots.len()),
            Err(error) => last = format!("database unavailable: {error:#}"),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for pane slots: {last}");
}

fn remove_owned_worktree(repo: &Path, path: &Path, branch: &str) {
    let remove = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "remove", "--force"])
        .arg(path)
        .output()
        .expect("remove exact worktree");
    assert!(
        remove.status.success(),
        "failed to remove exact worktree {}: {}",
        path.display(),
        String::from_utf8_lossy(&remove.stderr)
    );
    let branch_delete = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["branch", "-D", branch])
        .output()
        .expect("delete exact worktree branch");
    assert!(
        branch_delete.status.success(),
        "failed to delete exact branch {branch}: {}",
        String::from_utf8_lossy(&branch_delete.stderr)
    );
}

#[test]
#[serial]
fn new_session_persists_independent_pane_launch_config() {
    crate::harness::require_tmux!();

    let mut h = TuiTestHarness::new("pane_level_config");
    h.install_tool_stub("claude");
    h.install_tool_stub("codex");
    let repo = h.home_path().join("pane-repo");
    let worktree_root = h.home_path().join("owned-worktrees");
    create_repository(&repo);
    write_test_config(&h, &worktree_root);

    h.spawn_tui();
    h.wait_for("Agent of Empires");
    h.send_keys("n");
    h.wait_for("New Session");
    h.assert_screen_not_contains("Sandbox:");
    h.type_text("Pane Config");

    for _ in 0..3 {
        h.send_keys("Tab");
    }
    clear_focused_text(&h);
    h.type_text(repo.to_str().expect("UTF-8 repository path"));

    h.send_keys("Tab");
    h.send_keys("Space");
    h.send_keys("Tab");
    h.send_keys("Tab");
    h.type_text("left-pane");
    h.send_keys("Tab");
    select_codex_right_pane(&h);

    h.send_keys("Tab");
    h.type_text(repo.to_str().expect("UTF-8 repository path"));
    h.send_keys("Tab");
    h.send_keys("Tab");
    h.send_keys("Space");
    h.send_keys("Tab");
    h.type_text("right-pane");
    h.send_keys("Enter");

    let start = Instant::now();
    let instance_id = loop {
        if let Some(id) = session_id(&h, "Pane Config") {
            break id;
        }
        assert!(
            start.elapsed() <= Duration::from_secs(20),
            "timed out waiting for session creation\n{}",
            h.capture_screen()
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let session_name = agent_of_empires::tmux::Session::generate_name(&instance_id, "Pane Config");
    let slots = wait_for_two_slots(&h, &instance_id);

    assert_eq!(slots[0].slot, 0);
    assert_eq!(slots[0].agent, "claude");
    assert!(slots[0].yolo_mode);
    assert!(!slots[0].cross_agent_team);
    assert!(slots[0].identity_key.is_empty());
    assert_eq!(slots[0].cwd, worktree_root.join("left-pane"));
    assert!(slots[0].cwd.is_dir());
    assert!(slots[0]
        .worktree
        .worktree
        .as_ref()
        .is_some_and(|info| info.branch == "left-pane" && info.managed_by_aoe));

    assert_eq!(slots[1].slot, 1);
    assert_eq!(slots[1].agent, "codex");
    assert!(!slots[1].yolo_mode);
    assert!(slots[1].cross_agent_team);
    assert!(!slots[1].identity_key.is_empty());
    assert_eq!(slots[1].cwd, worktree_root.join("right-pane"));
    assert!(slots[1].cwd.is_dir());
    assert!(slots[1]
        .worktree
        .worktree
        .as_ref()
        .is_some_and(|info| info.branch == "right-pane" && info.managed_by_aoe));

    h.kill_tmux_target(&session_name);
    remove_owned_worktree(&repo, &slots[0].cwd, "left-pane");
    remove_owned_worktree(&repo, &slots[1].cwd, "right-pane");
}
