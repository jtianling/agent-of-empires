use std::time::{Duration, Instant};

use serial_test::serial;

use super::create_test_env_with_sessions;
use crate::session::{Instance, Status};
use crate::tui::restart_poller::{RestartIdentity, RestartResult};
use crate::tui::status_poller::StatusUpdate;

fn poll_result(instance: &Instance, status: Status) -> StatusUpdate {
    StatusUpdate {
        id: instance.id.clone(),
        last_start_time: instance.last_start_time,
        status,
        last_error: (status == Status::Error)
            .then(|| "agent exited; panes %83, %84 dropped to shell".to_string()),
        resume_token: Some("previous-conversation".to_string()),
        last_error_check: Some(Instant::now()),
        last_spinner_seen: None,
        spike_start: None,
        pre_spike_status: None,
        acknowledged: false,
    }
}

#[test]
#[serial]
fn stale_status_results_cannot_overwrite_completed_restart() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let id = env.view.selected_session.clone().expect("selected session");
    let now = Instant::now();

    for old_start in [None, Some(now - Duration::from_secs(10))] {
        for old_status in [Status::Error, Status::Idle] {
            env.view.mutate_instance(&id, |instance| {
                instance.last_start_time = old_start;
                instance.status = Status::Running;
            });
            let old_poll = poll_result(env.view.get_instance(&id).unwrap(), old_status);
            env.view.mutate_instance(&id, |instance| {
                instance.status = Status::Restarting;
                instance.restart_in_flight = true;
            });
            env.view.restart_poller.inject_result(RestartResult {
                session_id: id.clone(),
                identity: Some(RestartIdentity {
                    agent_session_id: None,
                    fork_pending: None,
                    resume_token: Some("current-conversation".to_string()),
                    xats_identity_key: None,
                    last_start_time: Some(now),
                }),
                last_error: Some(None),
                status: Status::Starting,
            });
            assert!(env.view.apply_restart_results());
            env.view.status_poller.inject_updates(vec![old_poll]);
            env.view.pending_status_refresh = true;
            assert!(env.view.apply_status_updates());

            let instance = env.view.get_instance(&id).unwrap();
            assert_eq!(instance.status, Status::Starting);
            assert_eq!(instance.last_error, None);
            assert_eq!(
                instance.resume_token.as_deref(),
                Some("current-conversation")
            );
            assert_eq!(instance.last_start_time, Some(now));
            assert!(!env.view.pending_status_refresh);
        }
    }
}

#[test]
#[serial]
fn current_status_results_still_report_a_fallen_agent() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let id = env.view.selected_session.clone().expect("selected session");
    env.view.mutate_instance(&id, |instance| {
        instance.status = Status::Idle;
        instance.last_start_time = Some(Instant::now() - Duration::from_secs(10));
    });
    let update = poll_result(env.view.get_instance(&id).unwrap(), Status::Error);
    env.view.status_poller.inject_updates(vec![update]);
    assert!(env.view.apply_status_updates());

    let instance = env.view.get_instance(&id).unwrap();
    assert_eq!(instance.status, Status::Error);
    assert!(instance.last_error.as_deref().unwrap().contains("%83, %84"));
    assert_eq!(
        instance.resume_token.as_deref(),
        Some("previous-conversation")
    );
}

#[test]
#[serial]
fn status_results_cannot_restore_tokens_during_restart() {
    crate::tmux::isolate_tmux_socket();
    let mut env = create_test_env_with_sessions(1);
    let id = env.view.selected_session.clone().expect("selected session");
    let update = poll_result(env.view.get_instance(&id).unwrap(), Status::Error);
    env.view.mutate_instance(&id, |instance| {
        instance.status = Status::Restarting;
        instance.restart_in_flight = true;
        instance.resume_token = None;
    });
    env.view.status_poller.inject_updates(vec![update]);
    assert!(env.view.apply_status_updates());

    let instance = env.view.get_instance(&id).unwrap();
    assert_eq!(instance.status, Status::Restarting);
    assert_eq!(instance.last_error, None);
    assert_eq!(instance.resume_token, None);
}
