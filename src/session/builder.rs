//! Instance creation and cleanup utilities.
//!
//! This module provides shared logic for building new session instances,
//! used by both synchronous (TUI operations) and asynchronous (background poller) code paths.

use std::path::PathBuf;

use anyhow::{bail, Result};
use chrono::Utc;

use crate::containers::{self, ContainerRuntimeInterface};
use crate::git::GitWorktree;

use super::{
    civilizations, Config, Instance, PaneConfig, PaneDraft, PaneWorktreeInfo, SandboxInfo,
    WorkspaceInfo, WorkspaceRepo, WorktreeInfo,
};

/// Parameters for creating a new session instance.
#[derive(Debug, Clone)]
pub struct InstanceParams {
    pub title: String,
    pub group: String,
    pub primary: PaneDraft,
    pub sandbox: bool,
    /// The sandbox image to use. Required when sandbox is true.
    pub sandbox_image: String,
    /// Claude development-channels string for Cross Agent Team launches.
    pub cross_agent_team_channel: String,
    /// Additional environment entries for the container.
    /// `KEY` = pass through from host, `KEY=VALUE` = set explicitly.
    pub extra_env: Vec<String>,
    /// Extra arguments to append after the agent binary
    pub extra_args: String,
    /// Command override for the agent binary (replaces the default binary)
    pub command_override: String,
}

/// Result of building an instance, tracking what was created for cleanup purposes.
pub struct BuildResult {
    pub instance: Instance,
    /// Path to worktree if one was created and managed by aoe
    pub created_worktree: Option<CreatedWorktree>,
    /// Workspace worktrees created during build (for cleanup)
    pub created_workspace_worktrees: Vec<CreatedWorktree>,
}

/// Info about a worktree created during instance building.
pub struct CreatedWorktree {
    pub path: PathBuf,
    pub main_repo_path: PathBuf,
}

/// Result of creating a multi-repo workspace.
pub struct WorkspaceResult {
    pub workspace_info: WorkspaceInfo,
    pub created_worktrees: Vec<CreatedWorktree>,
    pub workspace_path: PathBuf,
}

pub struct PaneBuildResult {
    pub config: PaneConfig,
    pub created_worktree: Option<CreatedWorktree>,
    pub created_workspace_worktrees: Vec<CreatedWorktree>,
}

/// Create a multi-repo workspace with worktrees for each repository.
///
/// Validates repo paths, detects name collisions, creates worktrees inside
/// a shared workspace directory, and rolls back on any error.
pub fn create_workspace(
    primary_path: &std::path::Path,
    extra_repo_paths: &[PathBuf],
    branch: &str,
    create_new_branch: bool,
    workspace_template: &str,
) -> Result<WorkspaceResult> {
    let primary_main_repo = GitWorktree::find_main_repo(primary_path)?;
    let primary_git_wt = GitWorktree::new(primary_main_repo)?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let session_id_short = &session_id[..8];

    let workspace_path =
        primary_git_wt.compute_path(branch, workspace_template, session_id_short)?;
    let workspace_dir = workspace_path.to_string_lossy().to_string();
    if workspace_path.exists() {
        bail!("Workspace already exists at {}", workspace_path.display());
    }

    let all_repo_paths: Vec<PathBuf> = std::iter::once(primary_path.to_path_buf())
        .chain(
            extra_repo_paths
                .iter()
                .map(|r| r.canonicalize().unwrap_or_else(|_| r.clone())),
        )
        .collect();

    // Check for duplicate repo directory names
    let mut seen_names = std::collections::HashSet::new();
    for repo_path in &all_repo_paths {
        let name = repo_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repo".to_string());
        if !seen_names.insert(name.clone()) {
            bail!(
                "Duplicate repository name '{}' in workspace\n\
                 Tip: Rename one of the directories to avoid the collision",
                name
            );
        }
    }
    std::fs::create_dir_all(&workspace_path)?;

    let mut repos = Vec::new();
    let mut created_worktrees: Vec<CreatedWorktree> = Vec::new();

    let cleanup = |created: &[CreatedWorktree], ws_path: &std::path::Path| {
        let mut errors = Vec::new();
        for wt in created {
            match GitWorktree::new(wt.main_repo_path.clone()) {
                Ok(git_wt) => {
                    if let Err(error) = git_wt.remove_worktree(&wt.path, false) {
                        errors.push(format!("{}: {error}", wt.path.display()));
                    }
                }
                Err(error) => errors.push(format!("{}: {error}", wt.path.display())),
            }
        }
        if let Err(error) = std::fs::remove_dir_all(ws_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!("{}: {error}", ws_path.display()));
            }
        }
        errors
    };
    let cleanup_suffix = |errors: &[String]| {
        if errors.is_empty() {
            String::new()
        } else {
            format!(". Cleanup failed: {}", errors.join("; "))
        }
    };

    for repo_path in &all_repo_paths {
        if !GitWorktree::is_git_repo(repo_path) {
            let errors = cleanup(&created_worktrees, &workspace_path);
            bail!(
                "Path is not in a git repository: {}\n\
                 Tip: All --repo paths must be git repositories{}",
                repo_path.display(),
                cleanup_suffix(&errors)
            );
        }

        let main_repo_path_raw = match GitWorktree::find_main_repo(repo_path) {
            Ok(path) => path,
            Err(error) => {
                let errors = cleanup(&created_worktrees, &workspace_path);
                bail!(
                    "Failed to resolve repository {}: {}{}",
                    repo_path.display(),
                    error,
                    cleanup_suffix(&errors)
                );
            }
        };
        let main_repo_path = main_repo_path_raw
            .canonicalize()
            .unwrap_or(main_repo_path_raw);
        let git_wt = match GitWorktree::new(main_repo_path.clone()) {
            Ok(git) => git,
            Err(error) => {
                let errors = cleanup(&created_worktrees, &workspace_path);
                bail!(
                    "Failed to open repository {}: {}{}",
                    main_repo_path.display(),
                    error,
                    cleanup_suffix(&errors)
                );
            }
        };

        let repo_name = repo_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repo".to_string());

        let worktree_subdir = workspace_path.join(&repo_name);

        if let Err(e) = git_wt.create_worktree(branch, &worktree_subdir, create_new_branch) {
            let errors = cleanup(&created_worktrees, &workspace_path);
            bail!(
                "Failed to create worktree for {}: {}{}",
                repo_name,
                e,
                cleanup_suffix(&errors)
            );
        }

        created_worktrees.push(CreatedWorktree {
            path: worktree_subdir.clone(),
            main_repo_path: main_repo_path.clone(),
        });

        repos.push(WorkspaceRepo {
            name: repo_name,
            source_path: repo_path.to_string_lossy().to_string(),
            branch: branch.to_string(),
            worktree_path: worktree_subdir.to_string_lossy().to_string(),
            main_repo_path: main_repo_path.to_string_lossy().to_string(),
            managed_by_aoe: true,
        });
    }

    Ok(WorkspaceResult {
        workspace_info: WorkspaceInfo {
            branch: branch.to_string(),
            workspace_dir,
            repos,
            created_at: Utc::now(),
            cleanup_on_delete: true,
        },
        created_worktrees,
        workspace_path,
    })
}

pub fn resolve_pane_config(
    draft: PaneDraft,
    path_fallback: Option<&str>,
    profile: &str,
) -> Result<PaneBuildResult> {
    let config = super::profile_config::resolve_config(profile).unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        Config::default()
    });
    let source_path = draft.validate(path_fallback)?;
    let cross_agent_team = draft.cross_agent_team_enabled();
    let mut final_path = source_path
        .canonicalize()
        .unwrap_or_else(|_| source_path.clone());
    let worktree = draft.worktree.clone().normalized();
    let mut worktree_info = None;
    let mut workspace_info = None;
    let mut created_worktree = None;
    let mut created_workspace_worktrees = Vec::new();

    if let Some(branch) = worktree.branch.as_deref() {
        if !GitWorktree::is_git_repo(&source_path) {
            bail!("Path is not in a git repository");
        }
        if worktree.extra_repo_paths.is_empty() {
            let main_repo_path_raw = GitWorktree::find_main_repo(&source_path)?;
            let main_repo_path = main_repo_path_raw
                .canonicalize()
                .unwrap_or(main_repo_path_raw);
            let git_wt = GitWorktree::new(main_repo_path.clone())?;
            let template = if GitWorktree::is_bare_repo(&main_repo_path) {
                &config.worktree.bare_repo_path_template
            } else {
                &config.worktree.path_template
            };

            if !worktree.create_new_branch {
                if let Some(existing) = git_wt
                    .list_worktrees()?
                    .into_iter()
                    .find(|candidate| candidate.branch.as_deref() == Some(branch))
                {
                    final_path = existing.path;
                    worktree_info = Some(reused_worktree_info(branch, &main_repo_path));
                } else {
                    let id = uuid::Uuid::new_v4().to_string();
                    let path = git_wt.compute_path(branch, template, &id[..8])?;
                    if path.exists() && worktree.reuse_existing {
                        final_path = path;
                        worktree_info = Some(reused_worktree_info(branch, &main_repo_path));
                    } else {
                        git_wt.create_worktree(branch, &path, false)?;
                        final_path = path.clone();
                        created_worktree = Some(CreatedWorktree {
                            path,
                            main_repo_path: main_repo_path.clone(),
                        });
                        worktree_info = Some(managed_worktree_info(branch, &main_repo_path));
                    }
                }
            } else {
                let id = uuid::Uuid::new_v4().to_string();
                let path = git_wt.compute_path(branch, template, &id[..8])?;
                if path.exists() {
                    if !worktree.reuse_existing {
                        bail!("Worktree already exists at {}", path.display());
                    }
                    final_path = path;
                    worktree_info = Some(reused_worktree_info(branch, &main_repo_path));
                } else {
                    git_wt.create_worktree(branch, &path, true)?;
                    final_path = path.clone();
                    created_worktree = Some(CreatedWorktree {
                        path,
                        main_repo_path: main_repo_path.clone(),
                    });
                    worktree_info = Some(managed_worktree_info(branch, &main_repo_path));
                }
            }
        } else {
            let extra_paths: Vec<PathBuf> = worktree
                .extra_repo_paths
                .iter()
                .map(PathBuf::from)
                .collect();
            let result = create_workspace(
                &source_path,
                &extra_paths,
                branch,
                worktree.create_new_branch,
                &config.worktree.workspace_path_template,
            )?;
            final_path = result.workspace_path;
            workspace_info = Some(result.workspace_info);
            created_workspace_worktrees = result.created_worktrees;
        }
    }

    if !final_path.is_dir() {
        bail!("Pane path is not a directory: {}", final_path.display());
    }
    let worktree_path = worktree_info
        .as_ref()
        .map(|_| final_path.to_string_lossy().to_string());
    let worktree = if worktree_info.is_some() || workspace_info.is_some() {
        Some(PaneWorktreeInfo {
            worktree_path,
            worktree: worktree_info,
            workspace: workspace_info,
        })
    } else {
        None
    };
    let yolo_mode = draft.yolo_mode_enabled();
    let mut pane = PaneConfig::new(
        draft.tool,
        final_path.to_string_lossy().to_string(),
        yolo_mode,
        cross_agent_team,
    );
    pane.xats_team = draft.xats_team;
    pane.xats_agent_name = draft.xats_agent_name;
    pane.worktree = worktree;
    pane = pane.normalized();
    pane.validate()?;
    Ok(PaneBuildResult {
        config: pane,
        created_worktree,
        created_workspace_worktrees,
    })
}

fn managed_worktree_info(branch: &str, main_repo_path: &std::path::Path) -> WorktreeInfo {
    WorktreeInfo {
        branch: branch.to_string(),
        main_repo_path: main_repo_path.to_string_lossy().to_string(),
        managed_by_aoe: true,
        created_at: Utc::now(),
        cleanup_on_delete: true,
    }
}

fn reused_worktree_info(branch: &str, main_repo_path: &std::path::Path) -> WorktreeInfo {
    WorktreeInfo {
        branch: branch.to_string(),
        main_repo_path: main_repo_path.to_string_lossy().to_string(),
        managed_by_aoe: false,
        created_at: Utc::now(),
        cleanup_on_delete: false,
    }
}

pub fn cleanup_resolved_pane(result: &PaneBuildResult) -> Result<()> {
    let mut errors = Vec::new();
    if let Some(worktree) = &result.created_worktree {
        match GitWorktree::new(worktree.main_repo_path.clone()) {
            Ok(git) => {
                if let Err(error) = git.remove_worktree(&worktree.path, false) {
                    errors.push(format!("worktree {}: {error}", worktree.path.display()));
                }
            }
            Err(error) => errors.push(format!(
                "worktree repository {}: {error}",
                worktree.main_repo_path.display()
            )),
        }
    }
    for worktree in &result.created_workspace_worktrees {
        match GitWorktree::new(worktree.main_repo_path.clone()) {
            Ok(git) => {
                if let Err(error) = git.remove_worktree(&worktree.path, false) {
                    errors.push(format!(
                        "workspace worktree {}: {error}",
                        worktree.path.display()
                    ));
                }
            }
            Err(error) => errors.push(format!(
                "workspace repository {}: {error}",
                worktree.main_repo_path.display()
            )),
        }
    }
    if !result.created_workspace_worktrees.is_empty() {
        if let Some(workspace) = result.config.workspace_info() {
            if let Err(error) = std::fs::remove_dir(&workspace.workspace_dir) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    errors.push(format!(
                        "workspace directory {}: {error}",
                        workspace.workspace_dir
                    ));
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        bail!("Failed to roll back pane resources: {}", errors.join("; "))
    }
}

/// Build an instance with all setup (worktree resolution, sandbox config).
///
/// This does NOT start the instance or create Docker containers - that happens
/// separately via `instance.start()`. This separation allows for proper cleanup
/// if starting fails.
pub fn build_instance(
    params: InstanceParams,
    existing_titles: &[&str],
    profile: &str,
) -> Result<BuildResult> {
    if params.sandbox {
        let runtime = containers::get_container_runtime();
        if !runtime.is_available() {
            bail!("Container runtime is not installed. Please install Docker or Apple Container to use sandbox mode.");
        }
        if !runtime.is_daemon_running() {
            bail!("Container runtime daemon is not running. Please start Docker or Apple Container to use sandbox mode.");
        }
    }

    let config = super::profile_config::resolve_config(profile).unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        Config::default()
    });

    let resolved_primary = resolve_pane_config(params.primary.clone(), None, profile)?;
    let mut primary_config = resolved_primary.config.clone();
    if params.sandbox {
        primary_config.cross_agent_team = false;
    }
    let final_path = primary_config.working_dir.clone();

    let final_title = if params.title.is_empty() {
        civilizations::generate_random_title(existing_titles)
    } else {
        params.title.clone()
    };

    let mut instance = Instance::new(&final_title, &final_path);
    instance.group_path = params.group;
    instance.command = crate::agents::get_agent(&primary_config.tool)
        .filter(|a| a.set_default_command)
        .map(|a| a.binary.to_string())
        .unwrap_or_default();
    instance.set_primary_pane_config(primary_config);
    if instance.cross_agent_team {
        instance.cross_agent_team_channel = params.cross_agent_team_channel.clone();
    }

    // Apply agent_command_override and agent_extra_args from resolved config.
    // Per-session values from params take priority over config.
    if !params.command_override.is_empty() {
        instance.command = params.command_override;
    } else if let Some(cmd_override) = config
        .session
        .agent_command_override
        .get(&params.primary.tool)
    {
        if !cmd_override.is_empty() {
            instance.command = cmd_override.clone();
        }
    }
    // For terminal sessions, default to the user's shell
    if params.primary.tool == "shell" && instance.command.is_empty() {
        instance.command = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    }

    if !params.extra_args.is_empty() {
        instance.extra_args = params.extra_args;
    } else if let Some(extra) = config.session.agent_extra_args.get(&params.primary.tool) {
        if !extra.is_empty() {
            instance.extra_args = extra.clone();
        }
    }

    if params.sandbox {
        instance.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: None,
            image: params.sandbox_image.clone(),
            container_name: containers::DockerContainer::generate_name(&instance.id),
            created_at: None,
            extra_env: if params.extra_env.is_empty() {
                None
            } else {
                Some(params.extra_env.clone())
            },
            custom_instruction: config.sandbox.custom_instruction.clone(),
        });
    }

    Ok(BuildResult {
        instance,
        created_worktree: resolved_primary.created_worktree,
        created_workspace_worktrees: resolved_primary.created_workspace_worktrees,
    })
}

/// Clean up resources created during a failed or cancelled instance build.
///
/// This handles:
/// - Removing worktrees created by aoe
/// - Removing Docker containers
/// - Killing tmux sessions
pub fn cleanup_instance(
    instance: &Instance,
    created_worktree: Option<&CreatedWorktree>,
    created_workspace_worktrees: &[CreatedWorktree],
) {
    if let Some(wt) = created_worktree {
        if let Ok(git_wt) = GitWorktree::new(wt.main_repo_path.clone()) {
            if let Err(e) = git_wt.remove_worktree(&wt.path, false) {
                tracing::warn!("Failed to clean up worktree: {}", e);
            }
        }
    }

    // Workspace worktree cleanup
    for wt in created_workspace_worktrees {
        if let Ok(git_wt) = GitWorktree::new(wt.main_repo_path.clone()) {
            if let Err(e) = git_wt.remove_worktree(&wt.path, false) {
                tracing::warn!("Failed to clean up workspace worktree: {}", e);
            }
        }
    }
    // Clean up workspace directory if workspace was created
    if let Some(ws_info) = &instance.workspace_info {
        let _ = std::fs::remove_dir_all(&ws_info.workspace_dir);
    }

    if let Some(sandbox) = &instance.sandbox_info {
        if sandbox.enabled {
            let container = containers::DockerContainer::from_session_id(&instance.id);
            if container.exists().unwrap_or(false) {
                if let Err(e) = container.remove(true) {
                    tracing::warn!("Failed to clean up container: {}", e);
                }
            }
        }
    }

    let _ = instance.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_instance_keeps_the_resolved_primary_pane_authoritative() {
        let temp = tempfile::tempdir().unwrap();
        let result = build_instance(
            InstanceParams {
                title: "pane-config".to_string(),
                group: String::new(),
                primary: PaneDraft {
                    tool: "pi".to_string(),
                    path: temp.path().to_string_lossy().to_string(),
                    ..Default::default()
                },
                sandbox: false,
                sandbox_image: String::new(),
                cross_agent_team_channel: String::new(),
                extra_env: Vec::new(),
                extra_args: String::new(),
                command_override: String::new(),
            },
            &[],
            "default",
        )
        .unwrap();

        assert!(result.instance.primary_pane_config().yolo_mode);
        assert!(result.instance.yolo_mode);
    }

    fn repository() -> (tempfile::TempDir, PathBuf, GitWorktree) {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = temp.path().join("repo");
        let repo = git2::Repository::init(&repo_path).unwrap();
        std::fs::write(repo_path.join("README.md"), "fixture\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("AoE Test", "aoe@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
            .unwrap();
        drop(tree);
        drop(repo);
        let git = GitWorktree::new(repo_path.clone()).unwrap();
        (temp, repo_path, git)
    }

    fn resolved_worktree(
        repo_path: &std::path::Path,
        path: PathBuf,
        branch: &str,
        managed: bool,
    ) -> PaneBuildResult {
        PaneBuildResult {
            config: PaneConfig {
                tool: "claude".to_string(),
                working_dir: path.to_string_lossy().to_string(),
                yolo_mode: false,
                cross_agent_team: false,
                xats_team: String::new(),
                xats_agent_name: String::new(),
                worktree: Some(PaneWorktreeInfo {
                    worktree_path: Some(path.to_string_lossy().to_string()),
                    worktree: Some(if managed {
                        managed_worktree_info(branch, repo_path)
                    } else {
                        reused_worktree_info(branch, repo_path)
                    }),
                    workspace: None,
                }),
            },
            created_worktree: managed.then(|| CreatedWorktree {
                path,
                main_repo_path: repo_path.to_path_buf(),
            }),
            created_workspace_worktrees: Vec::new(),
        }
    }

    #[test]
    fn two_panes_can_own_different_worktrees() {
        let (_temp, repo_path, git) = repository();
        let left_path = repo_path.parent().unwrap().join("left-pane");
        let right_path = repo_path.parent().unwrap().join("right-pane");
        git.create_worktree("left-pane", &left_path, true).unwrap();
        git.create_worktree("right-pane", &right_path, true)
            .unwrap();

        let left = resolved_worktree(&repo_path, left_path.clone(), "left-pane", true);
        let right = resolved_worktree(&repo_path, right_path.clone(), "right-pane", true);
        assert_ne!(left.config.working_dir, right.config.working_dir);
        assert!(left_path.is_dir());
        assert!(right_path.is_dir());

        cleanup_resolved_pane(&left).unwrap();
        cleanup_resolved_pane(&right).unwrap();
        assert!(!left_path.exists());
        assert!(!right_path.exists());
    }

    #[test]
    fn rollback_removes_only_owned_worktree_and_keeps_reused_sibling() {
        let (_temp, repo_path, git) = repository();
        let owned_path = repo_path.parent().unwrap().join("owned-pane");
        let reused_path = repo_path.parent().unwrap().join("reused-pane");
        git.create_worktree("owned-pane", &owned_path, true)
            .unwrap();
        git.create_worktree("reused-pane", &reused_path, true)
            .unwrap();
        let owned = resolved_worktree(&repo_path, owned_path.clone(), "owned-pane", true);
        let reused = resolved_worktree(&repo_path, reused_path.clone(), "reused-pane", false);

        cleanup_resolved_pane(&owned).unwrap();
        assert!(!owned_path.exists());
        assert!(reused_path.is_dir());
        assert!(reused.created_worktree.is_none());
    }

    #[test]
    fn same_branch_conflict_is_reported_without_reusing_implicitly() {
        let (_temp, repo_path, git) = repository();
        let first = repo_path.parent().unwrap().join("same-first");
        let second = repo_path.parent().unwrap().join("same-second");
        git.create_worktree("same-branch", &first, true).unwrap();

        assert!(git.create_worktree("same-branch", &second, true).is_err());
        assert!(first.is_dir());
        assert!(!second.exists());
    }
}
