use super::{RestartIdentity, RestartPath, RestartPoller, RestartRequest, RestartResult};
use crate::session::Status;

impl RestartPoller {
    pub(super) fn perform_group_restart(mut request: RestartRequest) -> RestartResult {
        let result = group_restart_path(&request).and_then(|path| match path {
            Some(path) => Ok(Some(path)),
            None => request
                .instance
                .start_for_restart(request.mode)
                .map(|()| None),
        });
        match result {
            Ok(Some(RestartPath::Recover)) => Self::perform_recovery(request),
            Ok(Some(_)) => Self::perform_respawn(request),
            Ok(None) => {
                crate::tmux::refresh_session_cache();
                RestartResult {
                    session_id: request.session_id,
                    identity: Some(RestartIdentity::from_instance(&request.instance)),
                    last_error: Some(None),
                    status: request.instance.status,
                }
            }
            Err(error) => {
                tracing::error!(
                    "Failed to restart group member '{}': {}",
                    request.session_id,
                    error
                );
                RestartResult {
                    session_id: request.session_id,
                    identity: Some(RestartIdentity::from_instance(&request.instance)),
                    last_error: Some(Some(error.to_string())),
                    status: Status::Error,
                }
            }
        }
    }
}

fn group_restart_path(request: &RestartRequest) -> anyhow::Result<Option<RestartPath>> {
    if request.instance.tmux_session()?.exists() {
        return Ok(Some(RestartPath::Respawn));
    }
    let store = crate::db::Store::open_with_schema(&request.profile)?;
    let read = store.read_slots_for_instance_with_diagnostics(&request.session_id)?;
    if !read.slots.is_empty() {
        return Ok(Some(RestartPath::Recover));
    }
    anyhow::ensure!(
        read.skipped == 0,
        "No valid tracked panes for group restart"
    );
    Ok(None)
}
