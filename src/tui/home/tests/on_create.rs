use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serial_test::serial;

use super::create_test_env_with_sessions;
use crate::session::builder::CreatedWorktree;
use crate::session::{Status, Storage};
use crate::tui::home::operations::CreatedSession;
use crate::tui::home::HomeView;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    Command::new("git")
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .current_dir(repo)
        .status()
        .unwrap()
        .success()
}

fn wait_for_result(view: &mut HomeView) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(id) = view.apply_on_create_results() {
            return Some(id);
        }
        if view.on_create_runs.is_empty() {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("on_create run did not finish");
}

#[test]
#[serial]
fn on_create_runs_off_the_event_loop_and_then_hands_over_the_launch() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let dir = env._temp.path().to_path_buf();
    let id = env.view.instances[0].id.clone();
    env.view.mutate_instance(&id, |inst| {
        inst.project_path = dir.to_string_lossy().to_string();
    });

    let created = CreatedSession {
        id: id.clone(),
        worktree: None,
        workspace_worktrees: Vec::new(),
    };
    env.view.run_on_create_in_background(
        created,
        vec!["sleep 0.2; touch hook-ran".to_string()],
        None,
    );

    let listed = env.view.get_instance(&id).unwrap();
    assert_eq!(listed.status, Status::Restarting);
    assert!(listed.restart_in_flight);
    assert!(
        env.view.background_starts.contains(&id),
        "shown as starting"
    );

    assert_eq!(wait_for_result(&mut env.view), Some(id.clone()));
    assert!(dir.join("hook-ran").exists());
    assert!(!env.view.get_instance(&id).unwrap().restart_in_flight);
    assert!(!env.view.background_starts.contains(&id));
}

#[test]
#[serial]
fn failed_on_create_drops_the_session_its_worktree_and_new_branch() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let repo = env._temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    let worktree = env._temp.path().join("wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "hook-fails",
            worktree.to_str().unwrap(),
        ],
    );

    let id = env.view.instances[0].id.clone();
    env.view.mutate_instance(&id, |inst| {
        inst.project_path = worktree.to_string_lossy().to_string();
    });
    let created = CreatedSession {
        id: id.clone(),
        worktree: Some(CreatedWorktree {
            path: worktree.clone(),
            main_repo_path: repo.clone(),
            created_branch: Some("hook-fails".to_string()),
        }),
        workspace_worktrees: Vec::new(),
    };
    env.view
        .run_on_create_in_background(created, vec!["echo boom >&2; exit 3".to_string()], None);

    assert_eq!(wait_for_result(&mut env.view), None);
    assert!(env.view.get_instance(&id).is_none(), "session dropped");
    assert!(
        Storage::new("test").unwrap().load().unwrap().is_empty(),
        "and not persisted"
    );
    assert!(!worktree.exists());
    assert!(!branch_exists(&repo, "hook-fails"));
    let dialog = env.view.info_dialog.as_ref().expect("failure is surfaced");
    assert!(dialog.message().contains("boom"), "{}", dialog.message());
}
