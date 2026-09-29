use super::{RestartIdentity, RestartPoller, RestartRequest, RestartResult};
use crate::session::{PaneDraft, Status};

/// The launch a `Start` request performs.
#[derive(Debug, Clone)]
pub struct StartRequest {
    pub size: Option<(u16, u16)>,
    /// on_launch hooks already ran in the creation poller.
    pub skip_on_launch: bool,
    pub right_pane: Option<PaneDraft>,
}

impl RestartPoller {
    /// The StayOnHome body of starting a new session: launch it, then split in
    /// its right pane. A right pane that could not be created is reported on
    /// the session rather than failing it.
    pub(super) fn perform_start(request: RestartRequest) -> RestartResult {
        let RestartRequest {
            session_id,
            mut instance,
            profile,
            prev_status,
            start,
            ..
        } = request;
        let Some(start) = start else {
            return RestartResult {
                session_id,
                identity: None,
                last_error: Some(Some("start request without launch details".to_string())),
                status: Status::Error,
            };
        };

        let running = instance
            .tmux_session()
            .map(|session| session.exists())
            .unwrap_or(false);
        if running {
            return RestartResult {
                session_id,
                identity: None,
                last_error: None,
                status: prev_status,
            };
        }

        instance.status = Status::Starting;
        if let Err(e) = instance.start_with_size_opts(start.size, start.skip_on_launch) {
            tracing::error!("Failed to start session '{}': {}", session_id, e);
            return RestartResult {
                session_id,
                identity: Some(RestartIdentity::from_instance(&instance)),
                last_error: Some(Some(e.to_string())),
                status: Status::Error,
            };
        }

        let pane_error = start.right_pane.and_then(|pending| {
            crate::tui::managed_pane::launch_managed_pane(&instance, &pending, &profile)
                .err()
                .map(|detail| format!("Right pane not created: {detail}"))
        });
        crate::tmux::refresh_session_cache();
        RestartResult {
            session_id,
            identity: Some(RestartIdentity::from_instance(&instance)),
            last_error: Some(pane_error),
            status: Status::Starting,
        }
    }
}
