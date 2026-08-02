//! `agent-of-empires remove` command implementation

use anyhow::{bail, Result};
use clap::Args;

use crate::containers;
use crate::git::cleanup::remove_managed_worktree;
use crate::git::GitWorktree;
use crate::session::{Config, GroupTree, Instance, PaneConfig, Storage};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct RemoveArgs {
    /// Session ID or title to remove
    identifier: String,

    /// Delete worktree directory (default: keep worktree)
    #[arg(long = "delete-worktree")]
    delete_worktree: bool,

    /// Delete git branch after worktree removal (default: per config)
    #[arg(long = "delete-branch")]
    delete_branch: bool,

    /// Force worktree removal even with untracked/modified files
    #[arg(long)]
    force: bool,

    /// Keep container instead of deleting it (default: delete per config)
    #[arg(long = "keep-container")]
    keep_container: bool,
}

fn pane_configs_for_cleanup(profile: &str, inst: &Instance) -> Vec<PaneConfig> {
    let mut panes = vec![inst.primary_pane_config().clone()];
    match crate::db::Store::open_with_schema(profile)
        .and_then(|store| store.read_slots_for_instance(&inst.id))
    {
        Ok(slots) => panes.extend(slots.into_iter().map(|slot| slot.pane_config())),
        Err(error) => eprintln!("Warning: failed to read pane metadata: {error}"),
    }
    let mut seen = HashSet::new();
    panes.retain(|pane| {
        let key = serde_json::to_string(&pane.worktree).unwrap_or_default();
        seen.insert(key)
    });
    panes
}

fn remove_exact_worktree(inst: &Instance, path: &Path, main_repo: &Path, force: bool) -> bool {
    match GitWorktree::new(main_repo.to_path_buf()) {
        Ok(git) => match remove_managed_worktree(&git, path, main_repo, inst, force) {
            Ok(()) => {
                println!("  Worktree removed: {}", path.display());
                true
            }
            Err(errors) => {
                for error in errors {
                    eprintln!("Warning: {error}");
                }
                eprintln!(
                    "You may need to remove it manually with: git worktree remove {}",
                    path.display()
                );
                false
            }
        },
        Err(error) => {
            eprintln!("Warning: failed to access git repository: {error}");
            false
        }
    }
}

fn delete_exact_branch(main_repo: &Path, branch: &str) {
    match GitWorktree::new(main_repo.to_path_buf()) {
        Ok(git) => {
            if let Err(error) = git.delete_branch(branch) {
                eprintln!("Warning: failed to delete branch '{branch}': {error}");
            } else {
                println!("  Branch '{branch}' deleted");
            }
        }
        Err(error) => eprintln!("Warning: failed to access git repository: {error}"),
    }
}

fn cleanup_pane_worktrees(profile: &str, inst: &Instance, args: &RemoveArgs, config: &Config) {
    for pane in pane_configs_for_cleanup(profile, inst) {
        let Some(metadata) = pane.worktree.as_ref() else {
            continue;
        };
        if let Some(info) = metadata.worktree.as_ref() {
            let worktree_path = metadata.worktree_path();
            if !info.managed_by_aoe || !info.cleanup_on_delete {
                if info.managed_by_aoe {
                    if let Some(path) = worktree_path {
                        println!(
                            "Worktree preserved at: {} (cleanup_on_delete disabled)",
                            path.display()
                        );
                    }
                }
            } else if let Some(path) = worktree_path {
                let main_repo = PathBuf::from(&info.main_repo_path);
                let removed = args.delete_worktree
                    && remove_exact_worktree(inst, path, &main_repo, args.force);
                if !args.delete_worktree {
                    println!(
                        "Worktree preserved at: {} (use --delete-worktree to remove)",
                        path.display()
                    );
                }
                let delete_branch =
                    args.delete_branch || (removed && config.worktree.delete_branch_on_cleanup);
                if delete_branch && (!args.delete_worktree || removed) {
                    delete_exact_branch(&main_repo, &info.branch);
                }
            } else {
                eprintln!("Warning: managed pane worktree has no immutable cleanup path");
            }
        }
        let Some(workspace) = metadata
            .workspace
            .as_ref()
            .filter(|workspace| workspace.cleanup_on_delete)
        else {
            continue;
        };
        let mut all_owned_removed = args.delete_worktree;
        let mut all_repos_owned = true;
        for repo in &workspace.repos {
            if !repo.managed_by_aoe {
                all_repos_owned = false;
                continue;
            }
            let path = PathBuf::from(&repo.worktree_path);
            let main_repo = PathBuf::from(&repo.main_repo_path);
            let removed =
                args.delete_worktree && remove_exact_worktree(inst, &path, &main_repo, args.force);
            all_owned_removed &= removed;
            let delete_branch =
                args.delete_branch || (removed && config.worktree.delete_branch_on_cleanup);
            if delete_branch && (!args.delete_worktree || removed) {
                delete_exact_branch(&main_repo, &repo.branch);
            }
        }
        if all_owned_removed && all_repos_owned {
            let workspace_dir = PathBuf::from(&workspace.workspace_dir);
            if let Err(error) = std::fs::remove_dir(&workspace_dir) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!(
                        "Warning: failed to remove workspace directory {}: {error}",
                        workspace_dir.display()
                    );
                }
            }
        }
    }
}

pub async fn run(profile: &str, args: RemoveArgs) -> Result<()> {
    let storage = Storage::new(profile)?;
    let (instances, groups) = storage.load_with_groups()?;
    let config = crate::session::resolve_config(profile).unwrap_or_default();

    let mut found = false;
    let mut removed_title = String::new();
    let mut new_instances = Vec::with_capacity(instances.len());

    for inst in instances {
        if inst.id == args.identifier
            || inst.id.starts_with(&args.identifier)
            || inst.title == args.identifier
        {
            found = true;
            removed_title = inst.title.clone();

            cleanup_pane_worktrees(profile, &inst, &args, &config);

            // Capture the session's pane ids before killing it so the durable
            // store can purge their volatile capture rows.
            let session_name = crate::tmux::Session::generate_name(&inst.id, &inst.title);
            let pane_ids = crate::db::reconcile::session_pane_ids(&session_name);

            // Kill tmux session if it exists
            if let Ok(tmux_session) = crate::tmux::Session::new(&inst.id, &inst.title) {
                if tmux_session.exists() {
                    if let Err(e) = tmux_session.kill() {
                        eprintln!("Warning: failed to kill tmux session: {}", e);
                        eprintln!(
                            "Session removed from Agent of Empires but may still be running in tmux"
                        );
                    }
                }
            }

            // Purge the session's durable + volatile store records.
            crate::db::purge_session_records(profile, &inst.id, &pane_ids);

            // Container cleanup (if config allows and user didn't request --keep-container)
            if let Some(sandbox) = &inst.sandbox_info {
                if sandbox.enabled && !args.keep_container {
                    if config.sandbox.auto_cleanup {
                        let container = containers::DockerContainer::from_session_id(&inst.id);
                        if container.exists().unwrap_or(false) {
                            if let Err(e) = container.remove(true) {
                                eprintln!("Warning: failed to remove container: {}", e);
                            } else {
                                println!("  Container removed");
                            }
                        }
                    } else {
                        println!(
                            "Container preserved: {} (auto_cleanup disabled in config)",
                            sandbox.container_name
                        );
                    }
                } else if args.keep_container {
                    println!("Container preserved: {}", sandbox.container_name);
                }
            }
        } else {
            new_instances.push(inst);
        }
    }

    if !found {
        bail!(
            "Session not found in profile '{}': {}",
            storage.profile(),
            args.identifier
        );
    }

    // Rebuild group tree and save
    let group_tree = GroupTree::new_with_groups(&new_instances, &groups);
    storage.save_with_groups(&new_instances, &group_tree)?;

    println!(
        "  Removed session: {} (from profile '{}')",
        removed_title,
        storage.profile()
    );

    Ok(())
}
