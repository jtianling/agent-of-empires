//! Background deletion handler for TUI responsiveness

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use crate::containers::DockerContainer;
use crate::git::cleanup::remove_managed_worktree;
use crate::git::GitWorktree;
use crate::session::{Instance, PaneConfig, PaneWorktreeInfo};

#[derive(Clone)]
pub struct DeletionRequest {
    pub session_id: String,
    pub instance: Instance,
    pub profile: String,
    pub delete_worktree: bool,
    pub delete_branch: bool,
    pub delete_sandbox: bool,
    pub force_delete: bool,
}

#[derive(Debug)]
pub struct DeletionResult {
    pub session_id: String,
    pub success: bool,
    pub error: Option<String>,
}

pub struct DeletionPoller {
    request_tx: mpsc::Sender<DeletionRequest>,
    result_rx: mpsc::Receiver<DeletionResult>,
    _handle: thread::JoinHandle<()>,
}

impl DeletionPoller {
    pub fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<DeletionRequest>();
        let (result_tx, result_rx) = mpsc::channel::<DeletionResult>();

        let handle = thread::spawn(move || {
            Self::deletion_loop(request_rx, result_tx);
        });

        Self {
            request_tx,
            result_rx,
            _handle: handle,
        }
    }

    fn deletion_loop(
        request_rx: mpsc::Receiver<DeletionRequest>,
        result_tx: mpsc::Sender<DeletionResult>,
    ) {
        while let Ok(request) = request_rx.recv() {
            let result = Self::perform_deletion(&request);
            if result_tx.send(result).is_err() {
                break;
            }
        }
    }

    fn perform_deletion(request: &DeletionRequest) -> DeletionResult {
        if request.instance.managed_xats.is_some() {
            return Self::perform_managed_deletion(request);
        }
        Self::perform_resources_deletion(request)
    }

    fn perform_managed_deletion(request: &DeletionRequest) -> DeletionResult {
        let result = Self::delete_managed(request);
        DeletionResult {
            session_id: request.session_id.clone(),
            success: result.is_ok(),
            error: result.err().map(|error| format!("{error:#}")),
        }
    }

    fn delete_managed(request: &DeletionRequest) -> anyhow::Result<()> {
        use crate::session::{GroupTree, Storage};
        use anyhow::{ensure, Context};

        let storage = Storage::new(&request.profile)?;
        let _operation = storage.operation_lock()?;
        let (mut instances, groups) = storage.load_with_groups()?;
        let index = instances
            .iter()
            .position(|item| item.id == request.session_id)
            .context("Managed session disappeared before deletion")?;
        let current = &instances[index];
        ensure!(
            same_managed_launch(current, &request.instance),
            "Managed session changed before deletion; reload and retry"
        );
        crate::managed_xats::prepare_release(&mut instances[index])?;
        storage.save_with_groups(&instances, &GroupTree::new_with_groups(&instances, &groups))?;
        let mut instance = instances[index].clone();
        instance.source_profile = request.profile.clone();
        instance.stop()?;
        crate::managed_xats::release(&instance)?;
        crate::managed_codex::cleanup(&instance)?;
        crate::managed_runtimes::cleanup(&instance)?;
        let exact = DeletionRequest {
            instance,
            ..request.clone()
        };
        let result = Self::perform_resources_deletion(&exact);
        ensure!(result.success, "{}", result.error.unwrap_or_default());
        instances.remove(index);
        storage.save_with_groups(&instances, &GroupTree::new_with_groups(&instances, &groups))
    }

    fn perform_resources_deletion(request: &DeletionRequest) -> DeletionResult {
        let mut errors = Vec::new();
        if request.delete_worktree || request.delete_branch {
            let pane_configs = Self::pane_configs_for_deletion(request, &mut errors);
            for pane in &pane_configs {
                Self::cleanup_pane_worktree(request, pane, &mut errors);
            }
        }

        // Container cleanup (if user opted to delete it)
        if request.delete_sandbox {
            if let Some(sandbox) = &request.instance.sandbox_info {
                if sandbox.enabled {
                    let container = DockerContainer::from_session_id(&request.instance.id);
                    if container.exists().unwrap_or(false) {
                        if let Err(e) = container.remove(true) {
                            errors.push(format!("Container: {}", e));
                        }
                    }
                }
            }
        }

        // Capture the session's pane ids before killing it so the durable
        // store can purge their volatile capture rows.
        let session_name =
            crate::tmux::Session::generate_name(&request.instance.id, &request.instance.title);
        let pane_ids = crate::db::reconcile::session_pane_ids(&session_name);

        // Tmux kill - non-fatal if session already gone
        let _ = request.instance.kill();

        // Clean up hook status files
        crate::hooks::cleanup_hook_status_dir(&request.instance.id);

        // Purge the session's durable + volatile store records.
        crate::db::purge_session_records(&request.profile, &request.instance.id, &pane_ids);

        DeletionResult {
            session_id: request.session_id.clone(),
            success: errors.is_empty(),
            error: if errors.is_empty() {
                None
            } else {
                Some(errors.join("; "))
            },
        }
    }

    fn pane_configs_for_deletion(
        request: &DeletionRequest,
        errors: &mut Vec<String>,
    ) -> Vec<PaneConfig> {
        let mut panes = vec![request.instance.primary_pane_config().clone()];
        match crate::db::Store::open_with_schema(&request.profile)
            .and_then(|store| store.read_slots_for_instance(&request.instance.id))
        {
            Ok(slots) => panes.extend(slots.into_iter().map(|slot| slot.pane_config())),
            Err(error) => errors.push(format!("Pane metadata: {error}")),
        }
        let mut seen = HashSet::new();
        panes.retain(|pane| {
            let key = serde_json::to_string(&pane.worktree).unwrap_or_default();
            seen.insert(key)
        });
        panes
    }

    fn cleanup_pane_worktree(
        request: &DeletionRequest,
        pane: &PaneConfig,
        errors: &mut Vec<String>,
    ) {
        let Some(PaneWorktreeInfo {
            worktree_path,
            worktree,
            workspace,
        }) = pane.worktree.as_ref()
        else {
            return;
        };
        if let Some(info) = worktree
            .as_ref()
            .filter(|info| info.managed_by_aoe && info.cleanup_on_delete)
        {
            let main_repo = PathBuf::from(&info.main_repo_path);
            let removed = match worktree_path
                .as_deref()
                .filter(|path| !path.trim().is_empty())
            {
                Some(_) if !request.delete_worktree => true,
                Some(path) => match GitWorktree::new(main_repo.clone()) {
                    Ok(git) => remove_managed_worktree(
                        &git,
                        &PathBuf::from(path),
                        &main_repo,
                        &request.instance,
                        request.force_delete,
                    )
                    .map_err(|pane_errors| {
                        errors.extend(
                            pane_errors
                                .into_iter()
                                .map(|error| format!("Pane worktree {path}: {error}")),
                        );
                    })
                    .is_ok(),
                    Err(error) => {
                        errors.push(format!("Pane worktree {path}: {error}"));
                        false
                    }
                },
                None => {
                    errors.push("Managed pane worktree has no immutable cleanup path".to_string());
                    false
                }
            };
            if request.delete_branch && removed {
                match GitWorktree::new(main_repo) {
                    Ok(git) => {
                        if let Err(error) = git.delete_branch(&info.branch) {
                            errors.push(format!("Branch {}: {error}", info.branch));
                        }
                    }
                    Err(error) => errors.push(format!("Branch {}: {error}", info.branch)),
                }
            }
        }
        let Some(workspace) = workspace.as_ref().filter(|info| info.cleanup_on_delete) else {
            return;
        };
        let all_repos_owned = workspace.repos.iter().all(|repo| repo.managed_by_aoe);
        let mut workspace_removed = true;
        for repo in workspace.repos.iter().filter(|repo| repo.managed_by_aoe) {
            let main_repo = PathBuf::from(&repo.main_repo_path);
            let removed = !request.delete_worktree
                || match GitWorktree::new(main_repo.clone()) {
                    Ok(git) => remove_managed_worktree(
                        &git,
                        &PathBuf::from(&repo.worktree_path),
                        &main_repo,
                        &request.instance,
                        request.force_delete,
                    )
                    .map_err(|repo_errors| {
                        errors.extend(
                            repo_errors
                                .into_iter()
                                .map(|error| format!("Workspace {}: {error}", repo.name)),
                        );
                    })
                    .is_ok(),
                    Err(error) => {
                        errors.push(format!("Workspace {}: {error}", repo.name));
                        false
                    }
                };
            workspace_removed &= removed;
            if request.delete_branch && removed {
                if let Ok(git) = GitWorktree::new(main_repo) {
                    if let Err(error) = git.delete_branch(&repo.branch) {
                        errors.push(format!("Branch {}: {error}", repo.branch));
                    }
                }
            }
        }
        if request.delete_worktree && workspace_removed && all_repos_owned {
            let path = PathBuf::from(&workspace.workspace_dir);
            if path.exists() {
                if let Err(error) = std::fs::remove_dir(path) {
                    errors.push(format!("Workspace dir: {error}"));
                }
            }
        }
    }

    pub fn request_deletion(&self, request: DeletionRequest) {
        let _ = self.request_tx.send(request);
    }

    pub fn try_recv_result(&self) -> Option<DeletionResult> {
        self.result_rx.try_recv().ok()
    }
}

fn same_managed_launch(current: &Instance, requested: &Instance) -> bool {
    let fence = |instance: &Instance| {
        instance
            .managed_xats
            .as_ref()
            .and_then(|identity| identity.claim.as_ref())
            .map(|claim| claim.fence.clone())
    };
    current.managed_xats.is_some()
        && current.xats_identity_key == requested.xats_identity_key
        && fence(current) == fence(requested)
}

impl Default for DeletionPoller {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn create_test_instance() -> Instance {
        Instance::new("Test Session", "/tmp/test-project")
    }

    #[test]
    fn managed_deletion_rejects_replaced_launch() {
        let mut old = create_test_instance();
        old.xats_identity_key = Some("key".to_string());
        old.managed_xats = Some(
            serde_json::from_value(serde_json::json!({
                "name": "worker", "team": "test",
                "launch_instance_id": "00000000-0000-4000-8000-000000000001",
                "claim": {"fence": {
                    "claim_id": "00000000-0000-4000-8000-000000000002",
                    "instance_id": "00000000-0000-4000-8000-000000000001",
                    "generation": 1
                }, "mcp_token": "private", "runtime": null}
            }))
            .unwrap(),
        );
        let mut current = old.clone();
        assert!(same_managed_launch(&current, &old));
        current
            .managed_xats
            .as_mut()
            .unwrap()
            .claim
            .as_mut()
            .unwrap()
            .fence
            .generation = 2;
        assert!(!same_managed_launch(&current, &old));
        assert!(!same_managed_launch(&create_test_instance(), &old));
    }

    #[test]
    fn test_deletion_result_success_when_no_worktree_or_sandbox() {
        let instance = create_test_instance();
        let request = DeletionRequest {
            session_id: instance.id.clone(),
            instance,
            profile: "default".to_string(),
            delete_worktree: false,
            delete_branch: false,
            delete_sandbox: false,
            force_delete: false,
        };

        let result = DeletionPoller::perform_deletion(&request);

        assert!(result.success);
        assert!(result.error.is_none());
        assert_eq!(result.session_id, request.session_id);
    }

    #[test]
    fn test_deletion_result_success_even_with_delete_worktree_flag_when_no_worktree() {
        let instance = create_test_instance();
        let request = DeletionRequest {
            session_id: instance.id.clone(),
            instance,
            profile: "default".to_string(),
            delete_worktree: true,
            delete_branch: false,
            delete_sandbox: false,
            force_delete: false,
        };

        let result = DeletionPoller::perform_deletion(&request);

        assert!(result.success);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_deletion_poller_channel_communication() {
        let poller = DeletionPoller::new();
        let instance = create_test_instance();
        let session_id = instance.id.clone();

        poller.request_deletion(DeletionRequest {
            session_id: session_id.clone(),
            instance,
            profile: "default".to_string(),
            delete_worktree: false,
            delete_branch: false,
            delete_sandbox: false,
            force_delete: false,
        });

        let mut result = None;
        for _ in 0..50 {
            result = poller.try_recv_result();
            if result.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(result.is_some(), "Timed out waiting for deletion result");

        let result = result.unwrap();
        assert_eq!(result.session_id, session_id);
        assert!(result.success);
    }

    #[test]
    fn test_deletion_poller_try_recv_returns_none_when_empty() {
        let poller = DeletionPoller::new();
        assert!(poller.try_recv_result().is_none());
    }

    #[test]
    fn test_deletion_request_preserves_session_id() {
        let instance = create_test_instance();
        let custom_id = "custom-session-id-123".to_string();

        let request = DeletionRequest {
            session_id: custom_id.clone(),
            instance,
            profile: "default".to_string(),
            delete_worktree: false,
            delete_branch: false,
            delete_sandbox: false,
            force_delete: false,
        };

        let result = DeletionPoller::perform_deletion(&request);
        assert_eq!(result.session_id, custom_id);
    }
}
