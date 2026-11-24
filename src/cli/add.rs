//! `agent-of-empires add` command implementation

use anyhow::{bail, Result};
use clap::Args;
use std::path::{Path, PathBuf};

use crate::containers::{self, ContainerRuntimeInterface};
use crate::session::builder;
use crate::session::repo_config;
use crate::session::{civilizations, resolve_config, GroupTree, Instance, SandboxInfo, Storage};

#[derive(Args)]
pub struct AddArgs {
    /// Project directory (defaults to current directory)
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Session title (defaults to folder name)
    #[arg(short = 't', long)]
    title: Option<String>,

    /// Group path (defaults to parent folder)
    #[arg(short = 'g', long)]
    group: Option<String>,

    /// Command to run (e.g., 'claude' or any other supported agent)
    #[arg(short = 'c', long = "cmd")]
    command: Option<String>,

    /// Parent session (creates sub-session, inherits group)
    #[arg(short = 'P', long)]
    parent: Option<String>,

    /// Launch the session immediately after creating
    #[arg(short = 'l', long)]
    launch: bool,

    /// Create session in a git worktree for the specified branch
    #[arg(short = 'w', long = "worktree")]
    worktree_branch: Option<String>,

    /// Create a new branch (use with --worktree)
    #[arg(short = 'b', long = "new-branch")]
    create_branch: bool,

    /// Additional repositories for multi-repo workspace (use with --worktree)
    #[arg(long = "repo", short = 'r')]
    extra_repos: Vec<PathBuf>,

    /// Run session in Docker sandbox
    #[arg(short = 's', long)]
    sandbox: bool,

    /// Custom Docker image for sandbox (implies --sandbox)
    #[arg(long = "sandbox-image")]
    sandbox_image: Option<String>,

    /// Enable YOLO mode (skip permission prompts)
    #[arg(short = 'y', long)]
    yolo: bool,

    /// Enable Cross Agent Team for this host session
    #[arg(long, conflicts_with_all = ["sandbox", "sandbox_image"])]
    cross_agent_team: bool,

    /// Automatically trust repository hooks without prompting
    #[arg(long = "trust-hooks")]
    trust_hooks: bool,

    /// Extra arguments to append after the agent binary
    #[arg(long)]
    extra_args: Option<String>,

    /// Override the agent binary command
    #[arg(long)]
    cmd_override: Option<String>,

    /// Reuse an existing worktree instead of failing (use with --worktree)
    #[arg(long)]
    reuse_worktree: bool,
}

pub async fn run(profile: &str, args: AddArgs) -> Result<()> {
    let mut path = if args.path.as_os_str() == "." {
        std::env::current_dir()?
    } else {
        args.path.canonicalize()?
    };

    if !path.is_dir() {
        bail!("Path is not a directory: {}", path.display());
    }

    if !args.extra_repos.is_empty() && args.worktree_branch.is_none() {
        bail!("--repo requires --worktree to specify a branch\nTip: aoe add /path --repo /other -w branch-name");
    }

    let config = match resolve_config(profile) {
        Ok(config) => config,
        Err(error) if args.cross_agent_team => return Err(error),
        Err(error) => {
            tracing::warn!("Could not load profile {profile}: {error:#}");
            Default::default()
        }
    };
    let resolve_tool = if args.cross_agent_team {
        crate::agents::agent_from_command
    } else {
        crate::agents::resolve_tool_name
    };
    let tool = match args.command.as_deref() {
        Some(cmd) => resolve_tool(cmd).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown tool in command: {}\nSupported tools: {}",
                cmd,
                crate::agents::agent_names().join(", ")
            )
        })?,
        None => match config.session.default_tool.as_deref() {
            Some(cmd) if args.cross_agent_team => resolve_tool(cmd).ok_or_else(|| {
                anyhow::anyhow!("Unknown default tool for Cross Agent Team: {cmd}")
            })?,
            default => default
                .and_then(resolve_tool)
                .or_else(|| {
                    crate::tmux::AvailableTools::detect()
                        .available_list()
                        .first()
                        .copied()
                })
                .unwrap_or("claude"),
        },
    };
    if args.cross_agent_team {
        if !Instance::supports_cross_agent_team_tool(tool) {
            bail!("Cross Agent Team is unsupported for '{tool}'");
        }
        if config.sandbox.enabled_by_default {
            bail!("Cross Agent Team requires a host session; sandbox is enabled by default");
        }
    }

    let (command, extra_args) = launch_arguments(
        tool,
        args.command.as_deref(),
        args.extra_args.as_deref(),
        config
            .session
            .agent_extra_args
            .get(tool)
            .map(String::as_str),
    )?;

    let mut worktree_info_opt = None;
    let mut workspace_info_opt = None;

    if let Some(branch_raw) = &args.worktree_branch {
        use crate::git::GitWorktree;
        use crate::session::WorktreeInfo;
        use chrono::Utc;

        let branch = branch_raw.trim();

        if !args.extra_repos.is_empty() {
            let ws_result = builder::create_workspace(
                &path,
                &args.extra_repos,
                branch,
                args.create_branch,
                &config.worktree.workspace_path_template,
            )?;

            for repo in &ws_result.workspace_info.repos {
                println!(
                    "  Created worktree: {} -> {}",
                    repo.name, repo.worktree_path
                );
            }

            path = ws_result.workspace_path;
            workspace_info_opt = Some(ws_result.workspace_info);

            println!("✓ Workspace created successfully");
        } else {
            // Single worktree mode (existing logic)
            if !GitWorktree::is_git_repo(&path) {
                bail!("Path is not in a git repository\nTip: Navigate to a git repository first");
            }

            let main_repo_path = GitWorktree::find_main_repo(&path)?;
            let git_wt = GitWorktree::new(main_repo_path.clone())?;

            let session_id = uuid::Uuid::new_v4().to_string();
            let session_id_short = &session_id[..8];

            // Choose appropriate template based on repo type (bare vs regular)
            // Use main_repo_path (not path) to correctly detect bare repos when running from a worktree
            let template = if GitWorktree::is_bare_repo(&main_repo_path) {
                &config.worktree.bare_repo_path_template
            } else {
                &config.worktree.path_template
            };
            let worktree_path = git_wt.compute_path(branch, template, session_id_short)?;

            if worktree_path.exists() {
                if args.reuse_worktree {
                    println!("Reusing existing worktree at: {}", worktree_path.display());
                    path = worktree_path;
                    worktree_info_opt = Some(WorktreeInfo {
                        branch: branch.to_string(),
                        main_repo_path: main_repo_path.to_string_lossy().to_string(),
                        managed_by_aoe: false,
                        created_at: Utc::now(),
                        cleanup_on_delete: false,
                    });
                } else {
                    bail!(
                        "Worktree already exists at {}\nTip: Use 'aoe add --worktree {} --reuse-worktree' to reuse it",
                        worktree_path.display(),
                        branch
                    );
                }
            } else {
                println!("Creating worktree at: {}", worktree_path.display());
                git_wt.create_worktree(branch, &worktree_path, args.create_branch)?;

                path = worktree_path;

                worktree_info_opt = Some(WorktreeInfo {
                    branch: branch.to_string(),
                    main_repo_path: main_repo_path.to_string_lossy().to_string(),
                    managed_by_aoe: true,
                    created_at: Utc::now(),
                    cleanup_on_delete: true,
                });

                println!("✓ Worktree created successfully");
            }
        }
    }

    let storage = Storage::new(profile)?;
    let (mut instances, groups) = storage.load_with_groups()?;

    // Resolve parent session if specified
    let mut group_path = args.group.clone();
    let parent_id = if let Some(parent_ref) = &args.parent {
        let parent = super::resolve_session(parent_ref, &instances)?;
        if parent.is_sub_session() {
            bail!("Cannot create sub-session of a sub-session (single level only)");
        }
        group_path = Some(parent.group_path.clone());
        Some(parent.id.clone())
    } else {
        None
    };

    // Generate title
    let final_title = if let Some(title) = &args.title {
        let trimmed_title = title.trim();
        if is_duplicate_session(&instances, trimmed_title, path.to_str().unwrap_or("")) {
            println!(
                "Session already exists with same title and path: {}",
                trimmed_title
            );
            return Ok(());
        }
        trimmed_title.to_string()
    } else {
        let existing_titles: Vec<&str> = instances.iter().map(|i| i.title.as_str()).collect();
        civilizations::generate_random_title(&existing_titles)
    };

    let mut instance = Instance::new(&final_title, path.to_str().unwrap_or(""));

    if let Some(group) = &group_path {
        instance.group_path = group.trim().to_string();
    }

    if let Some(parent) = parent_id {
        instance.parent_session_id = Some(parent);
    }

    instance.tool = tool.to_owned();
    instance.command = command;
    instance.extra_args = extra_args;

    // Apply set_default_command for agents that need it (e.g., opencode, codex)
    if instance.command.is_empty() {
        instance.command = crate::agents::get_agent(&instance.tool)
            .filter(|a| a.set_default_command)
            .map(|a| a.binary.to_string())
            .unwrap_or_default();
    }

    if let Some(worktree_info) = worktree_info_opt {
        instance.worktree_info = Some(worktree_info);
    }

    if let Some(workspace_info) = workspace_info_opt {
        instance.workspace_info = Some(workspace_info);
    }

    instance.yolo_mode = args.yolo || config.session.yolo_mode_default;
    if args.cross_agent_team {
        instance.cross_agent_team = true;
        instance.cross_agent_team_channel = config.session.cross_agent_team_channel.clone();
    }
    instance.sync_primary_pane_from_legacy();

    if let Some(ref cmd) = args.cmd_override {
        instance.command = cmd.clone();
    } else if let Some(cmd_override) = config.session.agent_command_override.get(&instance.tool) {
        if !cmd_override.is_empty() {
            instance.command = cmd_override.clone();
        }
    }

    // Handle sandbox setup
    let use_sandbox = args.sandbox || args.sandbox_image.is_some();

    let runtime = containers::get_container_runtime();
    if use_sandbox || config.sandbox.enabled_by_default {
        if !runtime.is_available() {
            if use_sandbox {
                bail!(
                    "Container runtime is not installed or not accessible.\n\
                     Install Docker: https://docs.docker.com/get-docker/\n\
                     Or on macOS: Apple Container\n\
                     Tip: Use 'aoe add' without --sandbox to run directly on host"
                );
            }
        } else {
            let container_name = containers::DockerContainer::generate_name(&instance.id);
            let image = args
                .sandbox_image
                .as_ref()
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| runtime.effective_default_image());
            instance.sandbox_info = Some(SandboxInfo {
                enabled: true,
                container_id: None,
                image,
                container_name,
                created_at: None,
                extra_env: None,
                custom_instruction: config.sandbox.custom_instruction.clone(),
            });
        }
    }

    // Check for repository hooks
    let hook_result: Result<()> = (|| {
        match repo_config::check_hook_trust(&path) {
            Ok(repo_config::HookTrustStatus::NeedsTrust { hooks, hooks_hash }) => {
                let should_trust = if args.trust_hooks {
                    true
                } else {
                    println!("\nRepository hooks detected in .aoe/config.toml:");
                    if !hooks.on_create.is_empty() {
                        println!("  on_create:");
                        for cmd in &hooks.on_create {
                            println!("    {}", cmd);
                        }
                    }
                    if !hooks.on_launch.is_empty() {
                        println!("  on_launch:");
                        for cmd in &hooks.on_launch {
                            println!("    {}", cmd);
                        }
                    }
                    print!("\nTrust and run these hooks? [y/N] ");
                    use std::io::Write;
                    std::io::stdout().flush()?;
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    input.trim().eq_ignore_ascii_case("y")
                };

                if should_trust {
                    trust_and_run_on_create(&path, &hooks_hash, &hooks)?;
                } else {
                    println!("Hooks skipped (session created without running hooks)");
                }
            }
            Ok(repo_config::HookTrustStatus::Trusted(hooks)) => {
                if !hooks.on_create.is_empty() {
                    println!("Running on_create hooks...");
                    repo_config::execute_hooks(&hooks.on_create, &path)?;
                    println!("✓ on_create hooks completed");
                }
            }
            Ok(repo_config::HookTrustStatus::NoHooks) => {}
            Err(e) => {
                tracing::warn!("Failed to check repo hooks: {}", e);
            }
        }
        Ok(())
    })();

    if let Err(e) = hook_result {
        // Clean up worktree if we created one
        if let Some(ref wt_info) = instance.worktree_info {
            if wt_info.managed_by_aoe {
                if let Ok(git_wt) =
                    crate::git::GitWorktree::new(std::path::PathBuf::from(&wt_info.main_repo_path))
                {
                    let removed = git_wt.remove_worktree(&path, false).is_ok();
                    if removed && args.create_branch {
                        if let Err(err) = git_wt.delete_branch(&wt_info.branch) {
                            eprintln!("Failed to delete branch {}: {}", wt_info.branch, err);
                        }
                    }
                }
            }
        }
        // Clean up workspace worktrees if we created them
        if let Some(ref ws_info) = instance.workspace_info {
            for repo in &ws_info.repos {
                if repo.managed_by_aoe {
                    let wt_path = PathBuf::from(&repo.worktree_path);
                    let main_repo = PathBuf::from(&repo.main_repo_path);
                    if let Ok(git_wt) = crate::git::GitWorktree::new(main_repo) {
                        let removed = git_wt.remove_worktree(&wt_path, false).is_ok();
                        if removed && args.create_branch {
                            if let Err(err) = git_wt.delete_branch(&repo.branch) {
                                eprintln!("Failed to delete branch {}: {}", repo.branch, err);
                            }
                        }
                    }
                }
            }
            let _ = std::fs::remove_dir_all(&ws_info.workspace_dir);
        }
        return Err(e);
    }

    instances.push(instance.clone());

    // Rebuild group tree
    let mut group_tree = GroupTree::new_with_groups(&instances, &groups);
    if !instance.group_path.is_empty() {
        group_tree.create_group(&instance.group_path);
    }

    storage.save_with_groups(&instances, &group_tree)?;

    println!("✓ Added session: {}", final_title);
    println!("  Profile: {}", storage.profile());
    println!("  Path:    {}", path.display());
    println!("  Group:   {}", instance.group_path);
    println!("  ID:      {}", instance.id);
    if let Some(cmd) = &args.command {
        println!("  Cmd:     {}", cmd);
    }
    if let Some(parent) = &args.parent {
        println!("  Parent:  {}", parent);
    }
    if instance.sandbox_info.is_some() {
        println!("  Sandbox: enabled");
    }
    if instance.yolo_mode {
        println!("  YOLO:    enabled");
    }
    if let Some(ws) = &instance.workspace_info {
        println!("  Workspace: {} repos", ws.repos.len());
        for repo in &ws.repos {
            println!("    - {} ({})", repo.name, repo.worktree_path);
        }
    }

    if args.launch {
        let idx = instances
            .iter()
            .position(|i| i.id == instance.id)
            .expect("just added instance");
        instances[idx].start_with_size(crate::terminal::get_size())?;
        storage.save_with_groups(&instances, &group_tree)?;

        let tmux_session = crate::tmux::Session::new(&instance.id, &instance.title)?;
        crate::tmux::utils::setup_session_cycle_bindings(storage.profile());
        tmux_session.attach()?;
    } else {
        println!();
        println!("Next steps:");
        println!(
            "  agent-of-empires session start {}   # Start the session",
            final_title
        );
        println!("  agent-of-empires                         # Open TUI and press Enter to attach");
    }

    Ok(())
}

pub fn is_duplicate_session(instances: &[Instance], title: &str, path: &str) -> bool {
    let normalized_path = path.trim_end_matches('/');
    instances.iter().any(|inst| {
        let existing_path = inst.project_path.trim_end_matches('/');
        existing_path == normalized_path && inst.title == title
    })
}

fn trust_and_run_on_create(
    project_path: &Path,
    hooks_hash: &str,
    hooks: &crate::session::HooksConfig,
) -> Result<()> {
    repo_config::trust_repo(project_path, hooks_hash)?;
    println!("✓ Repository hooks trusted");
    if !hooks.on_create.is_empty() {
        println!("Running on_create hooks...");
        repo_config::execute_hooks(&hooks.on_create, project_path)?;
        println!("✓ on_create hooks completed");
    }
    Ok(())
}

fn launch_arguments(
    tool: &str,
    command: Option<&str>,
    extra: Option<&str>,
    defaults: Option<&str>,
) -> Result<(String, String)> {
    if tool == "antigravity" {
        return crate::agents::antigravity::launch_arguments(command, extra, defaults);
    }
    let custom = command
        .filter(|cmd| cmd.trim().contains(' '))
        .unwrap_or_default();
    Ok((
        custom.to_owned(),
        extra.or(defaults).unwrap_or_default().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_fixture(dir: &Path, binary: &str, version: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(binary);
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{version}' \"$AOE_ANTIGRAVITY_XATS\" \"$@\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn check_native_invocation(
        instance: &Instance,
        bin: &Path,
        version: &str,
        token: Option<&str>,
    ) {
        let cmd = instance.build_agent_command(token).unwrap();
        let outer = shell_words::split(&cmd).unwrap();
        let (_, invocation) = outer.last().unwrap().split_once("exec env ").unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &format!("exec /usr/bin/env {invocation}")])
            .env_clear()
            .env("PATH", bin)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut expected = vec![version, "0", "--dangerously-skip-permissions"];
        if let Some(id) = token {
            expected.extend(["--conversation", id]);
        }
        expected.extend(["--model", "test model", "--effort", "high"]);
        assert_eq!(
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    #[serial_test::serial]
    fn antigravity_cli_model_and_yolo_survive_exact_resume() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(temp.path());
        let id = "01234567-89ab-cdef-0123-456789abcdef";
        let bin = temp.path().join("default bin");
        install_fixture(&bin, "agy", "1.4.9");
        let alternate = temp.path().join("user's alternate bin");
        let agy = install_fixture(&alternate, "agy", "1.5.14");
        let antigravity = install_fixture(&alternate, "antigravity", "1.5.14");
        for (name, absolute) in [
            ("antigravity", false),
            ("agy", false),
            (agy.to_str().unwrap(), true),
            (antigravity.to_str().unwrap(), true),
        ] {
            let mut instance = Instance::new("test", "/tmp");
            let source = format!("{} --model 'test model'", shell_words::quote(name));
            instance.tool = crate::agents::resolve_tool_name(&source).unwrap().into();
            (instance.command, instance.extra_args) = launch_arguments(
                &instance.tool,
                Some(&source),
                Some("--effort high"),
                Some("--effort low"),
            )
            .unwrap();
            instance.yolo_mode = true;
            instance.sync_primary_pane_from_legacy();
            let mut saved: Instance =
                serde_json::from_value(serde_json::to_value(&instance).unwrap()).unwrap();
            let expected_command = if absolute {
                crate::session::environment::shell_escape(name)
            } else {
                String::new()
            };
            assert_eq!(saved.command, expected_command);
            assert!(!saved.has_command_override());
            let version = if absolute { "1.5.14" } else { "1.4.9" };
            for token in [None, Some(id)] {
                check_native_invocation(&saved, &bin, version, token);
            }
            saved.extra_args = "--continue".into();
            assert!(saved.build_agent_command(None).is_none());
            assert!(saved.build_agent_command(Some(id)).is_none());
        }
    }

    #[test]
    fn antigravity_cli_rejects_persistent_session_selectors_from_each_source() {
        for flag in [
            "--conversation uuid",
            "--conversation=uuid",
            "--continue",
            "-c",
            "-continue=true",
        ] {
            let cmd = format!("agy {flag}");
            for (command, extra, defaults) in [
                (Some(cmd.as_str()), None, None),
                (None, Some(flag), None),
                (None, None, Some(flag)),
            ] {
                let err = launch_arguments("antigravity", command, extra, defaults).unwrap_err();
                assert!(err.to_string().contains("selects a conversation"), "{err}");
            }
        }
        let (_, args) = launch_arguments(
            "antigravity",
            Some("agy --model 'test model'"),
            None,
            Some("--effort low"),
        )
        .unwrap();
        assert_eq!(
            shell_words::split(&args).unwrap(),
            ["--effort", "low", "--model", "test model"]
        );
    }
}
