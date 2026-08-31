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
    extract_resume_token, is_valid_resume_token, pane_agent_is_shell, Instance, Status,
    StatusUpdateOptions,
};

const FULL_CHECK_INTERVAL: Duration = Duration::from_secs(10);

/// Minimum interval between durable-store reconcile passes. Short enough that
/// captures are snapshotted within a second or so of arriving, long enough to
/// avoid spamming `tmux list-panes` on every poll tick.
const RECONCILE_INTERVAL: Duration = Duration::from_millis(750);

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
    pub fn new(profile: String) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<Vec<Instance>>();
        let (result_tx, result_rx) = mpsc::channel::<Vec<StatusUpdate>>();

        let handle = thread::spawn(move || {
            Self::polling_loop(request_rx, result_tx, profile);
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
        profile: String,
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
        // Throttle the durable-store reconciler independently of the per-instance
        // polling tiers: it must snapshot captures for every session (including
        // idle ones) but should not spam tmux on every tick.
        let mut last_reconcile = Instant::now() - RECONCILE_INTERVAL;

        while let Ok(instances) = request_rx.recv() {
            cycle_count = cycle_count.wrapping_add(1);

            if last_reconcile.elapsed() >= RECONCILE_INTERVAL {
                crate::db::reconcile::reconcile_all(&profile, &instances);
                last_reconcile = Instant::now();
            }

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

            // Slot rows for the shell-fallback check, opened once per cycle.
            // Without a readable store the check degrades to the primary
            // pane's tool.
            let store = if any_pollable {
                crate::db::Store::open_with_schema(&profile)
                    .map_err(|e| tracing::debug!("status poller: cannot open store: {}", e))
                    .ok()
            } else {
                None
            };

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

                // A fallen-agent error is a one-way latch: it clears through
                // the start/restart paths that reset instance errors, never by
                // the poller re-evaluating the pane into a healthy state.
                if inst.status == Status::Error && is_fallen_agent_error(inst.last_error.as_deref())
                {
                    next_previous_statuses.insert(inst.id.clone(), Status::Error);
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

                // A tracked agent pane running a plain shell is a dead agent
                // (the pane-died hook's fallback), whatever the content
                // detector read off the screen.
                if let Some(error) = detect_fallen_agent(&inst, &session_name, store.as_ref()) {
                    inst.status = Status::Error;
                    inst.last_error = Some(error);
                    inst.last_error_check = Some(now);
                }

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
                        let _ = crate::tmux::tmux_command()
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

/// Marker prefix of the error the shell-fallback check raises. The poller uses
/// it to recognize its own error on later cycles: such an error latches until
/// a start/restart path resets `last_error`, and is never re-evaluated back to
/// healthy by the poller itself.
const FALLEN_AGENT_ERROR_PREFIX: &str = "agent exited;";

fn is_fallen_agent_error(last_error: Option<&str>) -> bool {
    last_error.is_some_and(|error| error.starts_with(FALLEN_AGENT_ERROR_PREFIX))
}

/// One tracked pane's inputs to the shell-fallback check.
struct TrackedPaneObservation<'a> {
    /// tmux pane id named in the error message (e.g. `%9`).
    pane: &'a str,
    /// Agent the pane's `agent_slot` row records (the instance tool when no
    /// slot row exists).
    recorded_agent: &'a str,
    /// The pane's live `#{pane_current_command}` from the batch pane query.
    live_command: &'a str,
    /// A shell is this pane's correct state (a command override resolving to
    /// a shell); recorded shell agents are exempt via `recorded_agent`.
    shell_expected: bool,
}

/// The fallen-agent verdict: a tracked pane whose recorded agent is not a
/// shell but whose live process is one has fallen back through the pane-died
/// hook. Returns the instance error message naming every fallen pane, or
/// `None` when no pane has fallen or the launch grace window (during which
/// the wrapper legitimately reports a shell) is still open.
fn fallen_agent_error(
    within_start_grace: bool,
    panes: &[TrackedPaneObservation],
) -> Option<String> {
    if within_start_grace {
        return None;
    }
    let fallen: Vec<&str> = panes
        .iter()
        .filter(|pane| {
            !pane.shell_expected
                && !pane_agent_is_shell(pane.recorded_agent)
                && crate::tmux::utils::is_shell_command(pane.live_command)
        })
        .map(|pane| pane.pane)
        .collect();
    if fallen.is_empty() {
        return None;
    }
    let noun = if fallen.len() == 1 { "pane" } else { "panes" };
    Some(format!(
        "{FALLEN_AGENT_ERROR_PREFIX} {noun} {} dropped to shell (restart with r/R or c/C)",
        fallen.join(", ")
    ))
}

/// Join each of the instance's `agent_slot` rows to the live pane it records
/// (or the primary pane to the instance tool when no slot rows exist) and
/// return the fallen-agent verdict. Uses only data this cycle already has:
/// slot rows and the cached batch pane info, no new tmux round-trips.
fn detect_fallen_agent(
    inst: &Instance,
    session_name: &str,
    store: Option<&crate::db::Store>,
) -> Option<String> {
    if matches!(
        inst.status,
        Status::Starting | Status::Stopped | Status::Deleting | Status::Restarting
    ) {
        return None;
    }
    let panes = crate::tmux::get_all_cached_pane_infos(session_name)?;
    let slots = store
        .and_then(|store| store.read_slots_for_instance(&inst.id).ok())
        .unwrap_or_default();

    let mut observations = Vec::new();
    if slots.is_empty() {
        if let Some(info) = panes.first().filter(|info| !info.is_dead) {
            observations.push(TrackedPaneObservation {
                pane: &info.pane_id,
                recorded_agent: &inst.tool,
                live_command: &info.current_command,
                shell_expected: inst.expects_shell(),
            });
        }
    } else {
        for slot in &slots {
            if slot.tmux_pane.is_empty() {
                continue;
            }
            let Some(info) = panes
                .iter()
                .find(|pane| pane.pane_id == slot.tmux_pane && !pane.is_dead)
            else {
                continue;
            };
            observations.push(TrackedPaneObservation {
                pane: &info.pane_id,
                recorded_agent: &slot.agent,
                live_command: &info.current_command,
                shell_expected: slot.slot == 0 && inst.expects_shell(),
            });
        }
    }
    fallen_agent_error(inst.within_start_grace_period(), &observations)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn obs<'a>(
        pane: &'a str,
        recorded_agent: &'a str,
        live_command: &'a str,
        shell_expected: bool,
    ) -> TrackedPaneObservation<'a> {
        TrackedPaneObservation {
            pane,
            recorded_agent,
            live_command,
            shell_expected,
        }
    }

    // Scenario: fallen codex pane surfaces as an error naming the pane and
    // the restart keys.
    #[test]
    fn test_fallen_codex_pane_is_an_error_naming_pane_and_restart_keys() {
        let error = fallen_agent_error(false, &[obs("%9", "codex", "zsh", false)])
            .expect("a codex slot running zsh has fallen");
        assert!(error.contains("%9"), "{error}");
        assert!(error.contains("r/R"), "{error}");
        assert!(error.contains("c/C"), "{error}");
    }

    // Scenario: multiple fallen panes are reported together, not just the
    // first one detected.
    #[test]
    fn test_multiple_fallen_panes_are_reported_together() {
        let error = fallen_agent_error(
            false,
            &[
                obs("%9", "codex", "zsh", false),
                obs("%12", "claude", "-bash", false),
            ],
        )
        .expect("both panes have fallen");
        assert!(error.contains("%9"), "{error}");
        assert!(error.contains("%12"), "{error}");
    }

    // A mixed instance reports only the fallen pane, not its healthy or
    // shell siblings.
    #[test]
    fn test_only_the_fallen_pane_is_named() {
        let error = fallen_agent_error(
            false,
            &[
                obs("%1", "codex", "codex", false),
                obs("%2", "shell", "zsh", false),
                obs("%3", "codex", "zsh", false),
            ],
        )
        .expect("%3 has fallen");
        assert!(error.contains("%3"), "{error}");
        assert!(!error.contains("%1"), "{error}");
        assert!(!error.contains("%2"), "{error}");
    }

    // Scenario: shell tools and shell slots are exempt -- a shell in the pane
    // is their correct state.
    #[test]
    fn test_shell_tools_and_shell_slots_are_exempt() {
        assert!(fallen_agent_error(false, &[obs("%1", "shell", "zsh", false)]).is_none());
        assert!(fallen_agent_error(false, &[obs("%2", "bash", "bash", false)]).is_none());
    }

    // Scenario: command overrides naming a shell are exempt.
    #[test]
    fn test_command_override_resolving_to_a_shell_is_exempt() {
        assert!(fallen_agent_error(false, &[obs("%1", "codex", "sh", true)]).is_none());
    }

    // Scenario: the launch window does not false-positive -- the wrapper
    // legitimately reports a shell until `exec` replaces it.
    #[test]
    fn test_start_grace_window_suppresses_detection() {
        assert!(fallen_agent_error(true, &[obs("%1", "codex", "zsh", false)]).is_none());
    }

    // A healthy agent pane (agent binary, or an interpreter like the codex
    // npm shim) is never a fallen agent.
    #[test]
    fn test_non_shell_live_commands_are_healthy() {
        assert!(fallen_agent_error(false, &[obs("%1", "codex", "codex", false)]).is_none());
        assert!(fallen_agent_error(false, &[obs("%2", "codex", "node", false)]).is_none());
        assert!(fallen_agent_error(false, &[]).is_none());
    }

    // Scenario: the error clears only through start/restart resets. The latch
    // must recognize its own message and nothing else, so other errors keep
    // re-evaluating normally.
    #[test]
    fn test_latch_recognizes_only_the_fallen_agent_error() {
        let error = fallen_agent_error(false, &[obs("%9", "codex", "zsh", false)]).unwrap();
        assert!(is_fallen_agent_error(Some(&error)));
        assert!(!is_fallen_agent_error(Some("Container is not running")));
        assert!(!is_fallen_agent_error(None));
    }

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
