use crate::session::{Instance, PaneDraft};

/// Split a managed agent pane into a running session and record its durable
/// slot, so the pane is restartable and the key the launch minted has a home.
///
/// An unset directory falls back to the session's own here rather than when
/// the dialog was submitted. A worktree-backed session's directory is decided
/// during creation, so a snapshot would put the pane in the original
/// repository while the session went to the worktree. Returns why the pane
/// was not created, with every partial step rolled back.
pub(super) fn launch_managed_pane(
    inst: &Instance,
    pending: &PaneDraft,
    profile: &str,
) -> Result<(), String> {
    let session_name = crate::tmux::Session::generate_name(&inst.id, &inst.title);
    let profile = profile.to_string();
    let resolved = match crate::session::builder::resolve_pane_config(
        pending.clone(),
        Some(&inst.project_path),
        &profile,
    ) {
        Ok(resolved) => resolved,
        Err(error) => {
            return Err(format!("{error:#}"));
        }
    };
    let pane = &resolved.config;
    let cwd = pane.working_dir.as_str();

    // Splitting anyway would leave an empty pane the user has to close,
    // with nothing saying why it is empty.
    let launch = match inst.prepare_extra_pane_config_command(&profile, &session_name, pane) {
        Ok(launch) => launch,
        Err(error) => {
            let detail = append_pane_cleanup_error(
                format!("{error:#}"),
                crate::session::builder::cleanup_resolved_pane(&resolved),
            );
            return Err(detail);
        }
    };

    // The directory can be one the user typed, so a split that fails is
    // surfaced rather than logged: a pane that silently does not appear is
    // the failure mode a chosen directory introduces.
    let pane_id = match crate::tmux::split_window_right(
        &session_name,
        cwd,
        &launch.command,
        pane.tool != "shell",
    ) {
        Ok(pane_id) => pane_id,
        Err(e) => {
            let detail = append_pane_cleanup_error(
                format!("{e:#}"),
                inst.rollback_prepared_extra_pane(&profile, &launch),
            );
            let detail = append_pane_cleanup_error(
                detail,
                crate::session::builder::cleanup_resolved_pane(&resolved),
            );
            return Err(detail);
        }
    };

    // The key the launch minted lives on the pane's slot record, so every
    // later relaunch reuses it instead of handing xats a key no identity
    // holds. The pane's own directory lives there too, so a restart returns
    // it here rather than to the session's directory.
    let recorded = inst.record_launched_extra_pane(
        &profile,
        &session_name,
        &crate::db::reconcile::LaunchedPane {
            pane_id: &pane_id,
            config: pane,
            identity_key: &launch.identity_key,
            native_session_id: &launch.native_session_id,
            prepared_slot: launch.prepared_slot,
            prepared_generation: launch.prepared_generation,
        },
    );

    if let Err(e) = recorded {
        tracing::error!("{:#}", e);
        let detail = match crate::tmux::kill_pane_exact(&pane_id) {
            Ok(()) => format!("{e:#}"),
            Err(rollback_error) => {
                format!("{e:#}. Failed to roll back pane {pane_id}: {rollback_error:#}")
            }
        };
        let detail =
            append_pane_cleanup_error(detail, inst.rollback_prepared_extra_pane(&profile, &launch));
        let detail = append_pane_cleanup_error(
            detail,
            crate::session::builder::cleanup_resolved_pane(&resolved),
        );
        return Err(detail);
    }
    inst.auto_confirm_launched_pane(&pane_id, pane);
    Ok(())
}

fn append_pane_cleanup_error(detail: String, cleanup: anyhow::Result<()>) -> String {
    match cleanup {
        Ok(()) => detail,
        Err(error) => format!("{detail}. {error:#}"),
    }
}
