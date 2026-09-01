//! Background restart handler for TUI responsiveness
//!
//! StayOnHome restarts (`c`/`r` on the home view) run here instead of on the
//! event loop: the keypress enqueues a request, the single worker thread runs
//! the unchanged restart pipeline on a cloned `Instance`, and the mutated
//! fields travel back as a `RestartResult` that the event loop merges via
//! `HomeView::apply_restart_results`.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc;
use std::thread;

use super::app::{combine_pane_errors, skipped_slot_warning};
use crate::session::{Instance, PaneResumeOutcome, RestartMode, Status};

/// Which restart pipeline the worker runs for a request. Decided at enqueue
/// time: a live tmux session is respawned in place, a dead one with persisted
/// slots is cold-start recovered. The worker re-validates the recovery case
/// because the queue introduces a delay between decision and execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPath {
    Respawn,
    Recover,
}

pub struct RestartRequest {
    pub session_id: String,
    /// Snapshot of the instance at enqueue time. The pipeline mutates this
    /// clone; the mutated fields travel back in the result.
    pub instance: Instance,
    pub profile: String,
    pub mode: RestartMode,
    pub path: RestartPath,
    /// Status before the enqueue parked the instance in `Restarting`,
    /// restored when the request turns out to be stale.
    pub prev_status: Status,
}

/// Identity fields the restart pipeline mutates on the worker's clone.
#[derive(Debug, Clone)]
pub struct RestartIdentity {
    pub agent_session_id: Option<String>,
    pub fork_pending: Option<String>,
    pub resume_token: Option<String>,
    pub xats_identity_key: Option<String>,
    pub last_start_time: Option<std::time::Instant>,
}

impl RestartIdentity {
    fn from_instance(instance: &Instance) -> Self {
        Self {
            agent_session_id: instance.agent_session_id.clone(),
            fork_pending: instance.fork_pending.clone(),
            resume_token: instance.resume_token.clone(),
            xats_identity_key: instance.xats_identity_key.clone(),
            last_start_time: instance.last_start_time,
        }
    }
}

#[derive(Debug)]
pub struct RestartResult {
    pub session_id: String,
    /// `None` when the pipeline never ran on the clone (stale request, panic,
    /// pre-pipeline failure): the live instance keeps its identity fields and
    /// nothing needs persisting.
    pub identity: Option<RestartIdentity>,
    /// `Some(update)` replaces the instance's `last_error` (an inner `None`
    /// clears it); `None` leaves it untouched.
    pub last_error: Option<Option<String>>,
    /// Final status applied along with clearing the in-flight flag.
    pub status: Status,
}

pub struct RestartPoller {
    request_tx: mpsc::Sender<RestartRequest>,
    result_rx: mpsc::Receiver<RestartResult>,
    #[cfg(test)]
    result_tx: mpsc::Sender<RestartResult>,
    _handle: thread::JoinHandle<()>,
}

impl RestartPoller {
    pub fn new() -> Self {
        Self::with_runner(Self::perform_restart)
    }

    fn with_runner<F>(run: F) -> Self
    where
        F: Fn(RestartRequest) -> RestartResult + Send + 'static,
    {
        let (request_tx, request_rx) = mpsc::channel::<RestartRequest>();
        let (result_tx, result_rx) = mpsc::channel::<RestartResult>();

        #[cfg(test)]
        let test_result_tx = result_tx.clone();

        let handle = thread::spawn(move || {
            Self::restart_loop(request_rx, result_tx, run);
        });

        Self {
            request_tx,
            result_rx,
            #[cfg(test)]
            result_tx: test_result_tx,
            _handle: handle,
        }
    }

    /// A panicking pipeline must still deliver a result: the apply step is the
    /// only thing that clears `restart_in_flight`, so a swallowed panic would
    /// park the instance in `Restarting` forever.
    fn restart_loop<F>(
        request_rx: mpsc::Receiver<RestartRequest>,
        result_tx: mpsc::Sender<RestartResult>,
        run: F,
    ) where
        F: Fn(RestartRequest) -> RestartResult,
    {
        while let Ok(request) = request_rx.recv() {
            let session_id = request.session_id.clone();
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| run(request)))
                .unwrap_or_else(|payload| RestartResult {
                    session_id,
                    identity: None,
                    last_error: Some(Some(format!(
                        "restart worker panicked: {}",
                        panic_message(payload.as_ref())
                    ))),
                    status: Status::Error,
                });
            if result_tx.send(result).is_err() {
                break;
            }
        }
    }

    fn perform_restart(request: RestartRequest) -> RestartResult {
        match request.path {
            RestartPath::Respawn => Self::perform_respawn(request),
            RestartPath::Recover => Self::perform_recovery(request),
        }
    }

    /// The StayOnHome body of `Action::RespawnAgentPane`, unchanged except that
    /// it runs on the worker's clone and reports instead of mutating the view.
    fn perform_respawn(request: RestartRequest) -> RestartResult {
        let RestartRequest {
            session_id,
            mut instance,
            profile,
            mode,
            ..
        } = request;

        // Distinguish "no tracked panes" from "could not read the store". A
        // read failure is not an empty slot set: degrade to a primary-pane
        // restart but surface the failure instead of silently narrowing the
        // restart scope.
        let (slots, slot_read_error) = match crate::db::Store::open_with_schema(&profile)
            .and_then(|store| store.read_slots_for_instance_with_diagnostics(&session_id))
        {
            Ok(read) => {
                let warning = skipped_slot_warning(read.skipped);
                (read.slots, warning)
            }
            Err(e) => {
                tracing::error!("Failed to read agent slots for '{}': {}", session_id, e);
                (
                    Vec::new(),
                    Some(format!(
                        "Could not read tracked panes: {e}; restarted primary pane only"
                    )),
                )
            }
        };

        if slots.is_empty() {
            // No tracked panes (or unreadable store): restart the primary
            // @aoe_agent_pane with the single-pane behavior.
            let respawn_result = match mode {
                RestartMode::Resume => instance.respawn_agent_pane(),
                RestartMode::Fresh => instance.respawn_agent_pane_fresh(),
            };
            if let Err(e) = respawn_result {
                tracing::error!("Failed to respawn agent pane: {}", e);
                return RestartResult {
                    session_id,
                    identity: Some(RestartIdentity::from_instance(&instance)),
                    last_error: Some(Some(e.to_string())),
                    status: Status::Error,
                };
            }
            crate::tmux::refresh_session_cache();
            return RestartResult {
                session_id,
                identity: Some(RestartIdentity::from_instance(&instance)),
                last_error: Some(slot_read_error),
                status: instance.status,
            };
        }

        // Fan out to every tracked pane, each resumed from its own persisted
        // native_session_id. Per-pane failures are recorded but do not abort
        // sibling restarts.
        let mut slots = slots;
        let mut identity_origins = HashMap::new();
        if let Ok(store) = crate::db::Store::open_with_schema(&profile) {
            identity_origins = instance.ensure_slot_identity_keys(&store, &mut slots);
        }

        let outcomes = instance.resume_all_tracked_panes(&slots, mode, &identity_origins);
        let restart_error = pane_error_summary(&outcomes, "restart");
        crate::tmux::refresh_session_cache();

        RestartResult {
            session_id,
            identity: Some(RestartIdentity::from_instance(&instance)),
            last_error: Some(combine_pane_errors(slot_read_error, restart_error)),
            status: instance.status,
        }
    }

    /// The StayOnHome body of `recover_instance`, unchanged except that it runs
    /// on the worker's clone and reports instead of mutating the view.
    fn perform_recovery(request: RestartRequest) -> RestartResult {
        let RestartRequest {
            session_id,
            mut instance,
            profile,
            mode,
            prev_status,
            ..
        } = request;

        let store = match crate::db::Store::open_with_schema(&profile) {
            Ok(store) => store,
            Err(e) => {
                tracing::error!(
                    "Failed to open store for recovery of '{}': {}",
                    session_id,
                    e
                );
                return RestartResult {
                    session_id,
                    identity: None,
                    last_error: Some(Some(e.to_string())),
                    status: prev_status,
                };
            }
        };
        let read = match store.read_slots_for_instance_with_diagnostics(&session_id) {
            Ok(read) => read,
            Err(e) => {
                tracing::error!(
                    "Failed to read slots for recovery of '{}': {}",
                    session_id,
                    e
                );
                return RestartResult {
                    session_id,
                    identity: None,
                    last_error: Some(Some(e.to_string())),
                    status: prev_status,
                };
            }
        };
        let slot_warning = skipped_slot_warning(read.skipped);
        let mut slots = read.slots;

        // Re-check recoverability at execution time: the queue delay means the
        // session may have come back alive or the slots may be gone. A stale
        // request completes as a no-op that restores the pre-enqueue status.
        if !instance.is_recoverable(!slots.is_empty()) {
            return RestartResult {
                session_id,
                identity: None,
                last_error: slot_warning.is_some().then_some(slot_warning),
                status: prev_status,
            };
        }

        let identity_origins = instance.ensure_slot_identity_keys(&store, &mut slots);

        match instance.recover_from_slots(&store, &slots, mode, &identity_origins) {
            Ok(outcomes) => {
                let recovery_error = pane_error_summary(&outcomes, "recover");
                crate::tmux::refresh_session_cache();
                RestartResult {
                    session_id,
                    identity: Some(RestartIdentity::from_instance(&instance)),
                    last_error: Some(combine_pane_errors(slot_warning, recovery_error)),
                    status: instance.status,
                }
            }
            Err(e) => {
                tracing::error!("Failed to recover instance '{}': {}", session_id, e);
                RestartResult {
                    session_id,
                    identity: Some(RestartIdentity::from_instance(&instance)),
                    last_error: Some(combine_pane_errors(slot_warning, Some(e.to_string()))),
                    status: Status::Error,
                }
            }
        }
    }

    pub fn request_restart(&self, request: RestartRequest) {
        let _ = self.request_tx.send(request);
    }

    pub fn try_recv_result(&self) -> Option<RestartResult> {
        self.result_rx.try_recv().ok()
    }

    /// Test-only: push a result into the channel without running the worker,
    /// so apply-path tests never reach the real pipeline (which talks to tmux).
    #[cfg(test)]
    pub(crate) fn inject_result(&self, result: RestartResult) {
        let _ = self.result_tx.send(result);
    }
}

impl Default for RestartPoller {
    fn default() -> Self {
        Self::new()
    }
}

fn pane_error_summary(outcomes: &[PaneResumeOutcome], verb: &str) -> Option<String> {
    let errors: Vec<String> = outcomes
        .iter()
        .filter_map(|o| match o {
            PaneResumeOutcome::Error(e) => Some(e.clone()),
            _ => None,
        })
        .collect();
    (!errors.is_empty()).then(|| {
        format!(
            "{} pane(s) failed to {}: {}",
            errors.len(),
            verb,
            errors.join("; ")
        )
    })
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn recv_result(poller: &RestartPoller) -> RestartResult {
        for _ in 0..100 {
            if let Some(result) = poller.try_recv_result() {
                return result;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("Timed out waiting for restart result");
    }

    fn test_request(session_id: &str) -> RestartRequest {
        RestartRequest {
            session_id: session_id.to_string(),
            instance: Instance::new("Test Session", "/tmp/test-project"),
            profile: "default".to_string(),
            mode: RestartMode::Resume,
            path: RestartPath::Respawn,
            prev_status: Status::Idle,
        }
    }

    #[test]
    fn test_request_in_result_out_through_channels() {
        let poller = RestartPoller::with_runner(|request| RestartResult {
            session_id: request.session_id,
            identity: Some(RestartIdentity::from_instance(&request.instance)),
            last_error: Some(None),
            status: Status::Starting,
        });

        poller.request_restart(test_request("restart-session-1"));

        let result = recv_result(&poller);
        assert_eq!(result.session_id, "restart-session-1");
        assert_eq!(result.status, Status::Starting);
        assert!(result.identity.is_some());
    }

    #[test]
    fn test_panicking_worker_still_delivers_error_result() {
        let poller = RestartPoller::with_runner(|_request: RestartRequest| -> RestartResult {
            panic!("boom in the restart pipeline");
        });

        poller.request_restart(test_request("restart-session-2"));

        let result = recv_result(&poller);
        assert_eq!(result.session_id, "restart-session-2");
        assert_eq!(result.status, Status::Error);
        assert!(
            result.identity.is_none(),
            "a panicked pipeline must not touch identity fields"
        );
        let error = result
            .last_error
            .flatten()
            .expect("the panic must land in last_error");
        assert!(error.contains("boom in the restart pipeline"), "{error}");
    }

    #[test]
    fn test_worker_survives_a_panicked_request_and_serves_the_next() {
        let poller = RestartPoller::with_runner(|request: RestartRequest| {
            if request.session_id == "panics" {
                panic!("induced panic");
            }
            RestartResult {
                session_id: request.session_id,
                identity: None,
                last_error: None,
                status: Status::Starting,
            }
        });

        poller.request_restart(test_request("panics"));
        poller.request_restart(test_request("survives"));

        let first = recv_result(&poller);
        assert_eq!(first.session_id, "panics");
        assert_eq!(first.status, Status::Error);

        let second = recv_result(&poller);
        assert_eq!(second.session_id, "survives");
        assert_eq!(second.status, Status::Starting);
    }

    #[test]
    fn test_try_recv_returns_none_when_empty() {
        let poller = RestartPoller::new();
        assert!(poller.try_recv_result().is_none());
    }
}
