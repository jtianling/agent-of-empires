//! Background status polling for TUI performance
//!
//! This module provides non-blocking status updates for sessions by running
//! tmux subprocess calls in a background thread. Two optimizations reduce
//! per-cycle overhead:
//!
//! 1. **Batched metadata**: A single `tmux list-panes -a` call fetches pane
//!    metadata (dead flag, current command) for all sessions at once, replacing
//!    O(3N) per-instance `display-message` subprocesses with O(1).
//!
//! 2. **Adaptive polling tiers**: Sessions are polled at different frequencies
//!    based on their status. Hot (Running/Waiting/Starting) every cycle, Warm
//!    (Idle/Unknown) every 5 cycles, Cold (Error) every 60 cycles, Frozen
//!    (Stopped/Deleting) never.

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::session::{
    extract_resume_token, is_valid_resume_token, Instance, Status, StatusUpdateOptions,
};

const FULL_CHECK_INTERVAL: Duration = Duration::from_secs(10);

/// Adaptive polling intervals (in cycles). 0 = never poll.
const TIER_HOT: u64 = 1;
const TIER_WARM: u64 = 5;
const TIER_COLD: u64 = 60;

fn polling_tier(status: Status) -> u64 {
    match status {
        Status::Running | Status::Waiting | Status::Starting => TIER_HOT,
        Status::Idle | Status::Unknown => TIER_WARM,
        Status::Error => TIER_COLD,
        Status::Stopped | Status::Deleting | Status::Restarting => 0,
    }
}

/// Result of a status check for a single session
#[derive(Debug)]
pub struct StatusUpdate {
    pub id: String,
    pub status: Status,
    pub last_error: Option<String>,
    pub resume_token: Option<String>,
    pub last_error_check: Option<Instant>,
    pub last_spinner_seen: Option<Instant>,
    pub spike_start: Option<Instant>,
    pub pre_spike_status: Option<Status>,
    pub acknowledged: bool,
}

/// Background thread that polls session status without blocking the UI
pub struct StatusPoller {
    request_tx: mpsc::Sender<Vec<Instance>>,
    result_rx: mpsc::Receiver<Vec<StatusUpdate>>,
    _handle: thread::JoinHandle<()>,
}

impl StatusPoller {
    pub fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<Vec<Instance>>();
        let (result_tx, result_rx) = mpsc::channel::<Vec<StatusUpdate>>();

        let handle = thread::spawn(move || {
            Self::polling_loop(request_rx, result_tx);
        });

        Self {
            request_tx,
            result_rx,
            _handle: handle,
        }
    }

    fn polling_loop(
        request_rx: mpsc::Receiver<Vec<Instance>>,
        result_tx: mpsc::Sender<Vec<StatusUpdate>>,
    ) {
        let container_check_interval = Duration::from_secs(5);
        // Initialize to the past so the first check runs immediately
        let mut last_container_check = Instant::now() - container_check_interval;
        let mut container_states: HashMap<String, bool> = HashMap::new();
        let mut previous_statuses: HashMap<String, Status> = HashMap::new();
        let mut last_activity: HashMap<String, i64> = HashMap::new();
        let mut last_full_check: HashMap<String, Instant> = HashMap::new();
        // Track pane titles we've set for agents that don't manage their own,
        // so we only call tmux select-pane when the title actually changes.
        let mut managed_pane_titles: HashMap<String, String> = HashMap::new();
        // Start at TIER_COLD - 1 so the first wrapping_add produces TIER_COLD,
        // which is divisible by all tier intervals -- ensuring every session is
        // polled on the very first cycle.
        let mut cycle_count: u64 = TIER_COLD - 1;

        while let Ok(instances) = request_rx.recv() {
            cycle_count = cycle_count.wrapping_add(1);

            // Pre-scan: check if any instance would actually be polled this cycle.
            // If not, skip the batch subprocess calls entirely.
            let any_pollable = instances.iter().any(|inst| {
                let tier = polling_tier(inst.status);
                tier != 0 && cycle_count % tier == 0
            });

            if any_pollable {
                crate::tmux::refresh_session_cache();
                crate::tmux::refresh_pane_info_cache();
            }

            // Refresh container health if any sandboxed session exists and interval elapsed
            if any_pollable {
                let has_sandboxed = instances.iter().any(|i| i.is_sandboxed());
                if has_sandboxed && last_container_check.elapsed() >= container_check_interval {
                    container_states = crate::containers::batch_container_health();
                    last_container_check = Instant::now();
                }
            }

            let mut updates = Vec::with_capacity(instances.len());
            let mut next_previous_statuses = HashMap::with_capacity(instances.len());

            for mut inst in instances {
                // Adaptive polling: skip instances whose tier interval hasn't elapsed
                let tier = polling_tier(inst.status);
                if tier == 0 || cycle_count % tier != 0 {
                    continue;
                }

                let previous_status = previous_statuses.get(&inst.id).copied();
                let now = Instant::now();

                // For sandboxed sessions, check if the container is dead before
                // falling through to tmux-based status detection.
                if inst.is_sandboxed()
                    && !matches!(
                        inst.status,
                        Status::Stopped | Status::Deleting | Status::Starting | Status::Restarting
                    )
                {
                    if let Some(sandbox) = &inst.sandbox_info {
                        if let Some(&running) = container_states.get(&sandbox.container_name) {
                            if !running {
                                next_previous_statuses.insert(inst.id.clone(), Status::Error);
                                updates.push(StatusUpdate {
                                    id: inst.id,
                                    status: Status::Error,
                                    last_error: Some("Container is not running".to_string()),
                                    resume_token: None,
                                    last_error_check: inst.last_error_check,
                                    last_spinner_seen: inst.last_spinner_seen,
                                    spike_start: inst.spike_start,
                                    pre_spike_status: inst.pre_spike_status,
                                    acknowledged: inst.acknowledged,
                                });
                                continue;
                            }
                        }
                    }
                }

                let session_name = crate::tmux::Session::generate_name(&inst.id, &inst.title);
                let current_activity = crate::tmux::get_cached_window_activity(&session_name);
                let hook_based = crate::agents::get_agent(&inst.tool)
                    .is_some_and(|agent| agent.hook_config.is_some());
                let decision = decide_activity_gate(
                    hook_based,
                    current_activity,
                    last_activity.get(&inst.id).copied(),
                    last_full_check.get(&inst.id).copied(),
                    inst.spike_start.is_some(),
                    now,
                );

                if decision.activity_changed {
                    inst.acknowledged = false;
                }
                if let Some(activity) = current_activity {
                    last_activity.insert(inst.id.clone(), activity);
                }
                if !hook_based && !decision.skip_capture {
                    last_full_check.insert(inst.id.clone(), now);
                }

                inst.update_status_with_options(StatusUpdateOptions {
                    allow_capture: !decision.skip_capture,
                    reused_status: decision
                        .skip_capture
                        .then_some(previous_status.unwrap_or(inst.status)),
                });

                let resume_token = if previous_status != Some(Status::Error)
                    && inst.status == Status::Error
                {
                    crate::agents::get_agent(&inst.tool)
                        .and_then(|agent| agent.resume.as_ref())
                        .and_then(|resume| {
                            let output = inst.tmux_session().ok()?.capture_pane_cached(100).ok()?;
                            let token = extract_resume_token(&output, resume.resume_pattern)?;
                            if is_valid_resume_token(&token) {
                                Some(token)
                            } else {
                                tracing::warn!(
                                    "Ignoring invalid stored resume token for '{}': {:?}",
                                    inst.title,
                                    token
                                );
                                None
                            }
                        })
                } else {
                    None
                };

                // For agents that don't set their own title, keep the pane
                // title aligned with the session title. Codex is handled by
                // its dedicated tmux monitor so the dashboard poller does
                // not race the live waiting indicator.
                let agent_manages_title =
                    crate::agents::get_agent(&inst.tool).is_some_and(|a| a.sets_own_title);
                if !agent_manages_title && inst.tool != "codex" {
                    let desired = inst.title.clone();
                    let last = managed_pane_titles.get(&inst.id);
                    if last.map_or(true, |prev| *prev != desired) {
                        let session_name =
                            crate::tmux::Session::generate_name(&inst.id, &inst.title);
                        let _ = std::process::Command::new("tmux")
                            .args(["select-pane", "-t", &session_name, "-T", &desired])
                            .output();
                        managed_pane_titles.insert(inst.id.clone(), desired);
                    }
                }

                next_previous_statuses.insert(inst.id.clone(), inst.status);
                updates.push(StatusUpdate {
                    id: inst.id,
                    status: inst.status,
                    last_error: inst.last_error,
                    resume_token,
                    last_error_check: inst.last_error_check,
                    last_spinner_seen: inst.last_spinner_seen,
                    spike_start: inst.spike_start,
                    pre_spike_status: inst.pre_spike_status,
                    acknowledged: inst.acknowledged,
                });
            }

            previous_statuses = next_previous_statuses;

            if result_tx.send(updates).is_err() {
                break;
            }
        }
    }

    /// Request a status refresh for all given instances (non-blocking).
    pub fn request_refresh(&self, instances: Vec<Instance>) {
        let _ = self.request_tx.send(instances);
    }

    /// Try to receive status updates without blocking.
    /// Returns None if no updates are available yet.
    pub fn try_recv_updates(&self) -> Option<Vec<StatusUpdate>> {
        self.result_rx.try_recv().ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActivityGateDecision {
    activity_changed: bool,
    skip_capture: bool,
}

fn decide_activity_gate(
    hook_based: bool,
    current_activity: Option<i64>,
    last_activity: Option<i64>,
    last_full_check: Option<Instant>,
    spike_pending: bool,
    now: Instant,
) -> ActivityGateDecision {
    let activity_changed = match (current_activity, last_activity) {
        (Some(current), Some(previous)) => current != previous,
        (Some(_), None) => true,
        (None, _) => false,
    };
    let full_check_due = last_full_check
        .map(|last_check| now.duration_since(last_check) >= FULL_CHECK_INTERVAL)
        .unwrap_or(true);

    ActivityGateDecision {
        activity_changed,
        skip_capture: !hook_based
            && current_activity.is_some()
            && !activity_changed
            && !full_check_due
            && !spike_pending,
    }
}

impl Default for StatusPoller {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_gate_skips_when_activity_unchanged_and_recent() {
        let now = Instant::now();

        let decision = decide_activity_gate(false, Some(42), Some(42), Some(now), false, now);

        assert_eq!(
            decision,
            ActivityGateDecision {
                activity_changed: false,
                skip_capture: true,
            }
        );
    }

    #[test]
    fn test_activity_gate_forces_periodic_full_check() {
        let now = Instant::now();

        let decision = decide_activity_gate(
            false,
            Some(42),
            Some(42),
            Some(now - FULL_CHECK_INTERVAL),
            false,
            now,
        );

        assert_eq!(
            decision,
            ActivityGateDecision {
                activity_changed: false,
                skip_capture: false,
            }
        );
    }

    #[test]
    fn test_activity_gate_bypasses_hook_agents() {
        let now = Instant::now();

        let decision = decide_activity_gate(true, Some(42), Some(42), Some(now), false, now);

        assert_eq!(
            decision,
            ActivityGateDecision {
                activity_changed: false,
                skip_capture: false,
            }
        );
    }

    #[test]
    fn test_polling_tier_hot() {
        assert_eq!(polling_tier(Status::Running), TIER_HOT);
        assert_eq!(polling_tier(Status::Waiting), TIER_HOT);
        assert_eq!(polling_tier(Status::Starting), TIER_HOT);
    }

    #[test]
    fn test_polling_tier_warm() {
        assert_eq!(polling_tier(Status::Idle), TIER_WARM);
        assert_eq!(polling_tier(Status::Unknown), TIER_WARM);
    }

    #[test]
    fn test_polling_tier_cold() {
        assert_eq!(polling_tier(Status::Error), TIER_COLD);
    }

    #[test]
    fn test_polling_tier_frozen() {
        assert_eq!(polling_tier(Status::Stopped), 0);
        assert_eq!(polling_tier(Status::Deleting), 0);
    }

    #[test]
    fn test_tier_cycle_alignment() {
        // Hot sessions are polled every cycle
        assert_eq!(TIER_HOT, 1);
        // Warm sessions are polled every 5 cycles
        assert_ne!(1u64 % TIER_WARM, 0);
        assert_ne!(2u64 % TIER_WARM, 0);
        assert_eq!(5u64 % TIER_WARM, 0);
        assert_eq!(10u64 % TIER_WARM, 0);
        // Cold sessions are polled every 60 cycles
        assert_ne!(1u64 % TIER_COLD, 0);
        assert_eq!(60u64 % TIER_COLD, 0);
        assert_eq!(120u64 % TIER_COLD, 0);
    }

    #[test]
    fn test_first_cycle_polls_all_tiers() {
        // cycle_count starts at TIER_COLD - 1, first cycle wraps to TIER_COLD
        let first_cycle = (TIER_COLD - 1).wrapping_add(1);
        assert_eq!(TIER_HOT, 1, "hot tier must poll every cycle");
        assert_eq!(first_cycle % TIER_WARM, 0, "first cycle must poll warm");
        assert_eq!(first_cycle % TIER_COLD, 0, "first cycle must poll cold");
    }
}
