//! on_create hooks for sessions created with `n`. They run on their own thread
//! so the list stays usable (a hook such as `npm install` can take minutes and
//! must not hold up the restart worker either); the launch follows on the
//! background worker once they succeed.

use std::path::Path;
use std::sync::mpsc::{self, TryRecvError};

use crate::session::builder;
use crate::session::{repo_config, GroupTree, PaneDraft, Status};
use crate::tui::dialogs::InfoDialog;

use super::operations::CreatedSession;
use super::HomeView;

pub(super) struct OnCreateRun {
    right_pane: Option<PaneDraft>,
    result_rx: mpsc::Receiver<Result<(), String>>,
}

impl HomeView {
    /// Run `on_create` for a session that is already listed, showing it as
    /// starting meanwhile. A failure removes the session's worktrees (and the
    /// branches made for them) on the hook thread.
    pub(super) fn run_on_create_in_background(
        &mut self,
        created: CreatedSession,
        on_create: Vec<String>,
        right_pane: Option<PaneDraft>,
    ) {
        let Some(instance) = self.get_instance(&created.id).cloned() else {
            return;
        };
        let id = created.id.clone();
        let (result_tx, result_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = repo_config::execute_hooks(&on_create, Path::new(&instance.project_path))
                .map_err(|error| {
                    builder::cleanup_instance(
                        &instance,
                        created.worktree.as_ref(),
                        &created.workspace_worktrees,
                    );
                    format!("{error:#}")
                });
            if result_tx.send(result).is_err() {
                tracing::warn!("on_create result for '{}' had no receiver", instance.id);
            }
        });

        self.mutate_instance(&id, |inst| {
            inst.restart_in_flight = true;
            inst.status = Status::Restarting;
        });
        self.background_starts.insert(id.clone());
        self.on_create_runs.insert(
            id,
            OnCreateRun {
                right_pane,
                result_rx,
            },
        );
    }

    /// Apply one finished on_create run. Returns the session to launch when
    /// its hooks succeeded; a failed one is dropped from the list and the
    /// reason shown, since nothing of it is left to act on.
    pub fn apply_on_create_results(&mut self) -> Option<String> {
        let (id, result) =
            self.on_create_runs
                .iter()
                .find_map(|(id, run)| match run.result_rx.try_recv() {
                    Ok(result) => Some((id.clone(), result)),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => Some((
                        id.clone(),
                        Err("on_create hook thread exited without a result".to_string()),
                    )),
                })?;
        let run = self.on_create_runs.remove(&id)?;
        self.background_starts.remove(&id);
        self.mutate_instance(&id, |inst| inst.restart_in_flight = false);

        match result {
            Ok(()) => {
                self.pending_right_pane = run.right_pane;
                Some(id)
            }
            Err(error) => {
                tracing::error!("on_create hook failed for '{}': {}", id, error);
                self.instances.retain(|inst| inst.id != id);
                self.instance_map.remove(&id);
                self.group_tree = GroupTree::new_with_groups(&self.instances, &self.groups);
                if let Err(e) = self.save() {
                    tracing::error!("Failed to save after dropping '{}': {}", id, e);
                }
                self.info_dialog = Some(InfoDialog::new(
                    "Session Not Created",
                    &format!(
                        "on_create hook failed, so the session and its worktree were \
                         removed:\n\n{error}"
                    ),
                ));
                None
            }
        }
    }
}
