//! Background notification monitor for tmux status bar.
//!
//! Polls session statuses and updates per-session tmux user options so the
//! status bar can display Waiting/Running/Idle sessions.

mod captures;
mod inventory;
mod legacy_owner;
mod titles;

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::session::config::{load_config, SortOrder};
use crate::session::{
    expanded_groups, flatten_tree, Group, GroupTree, Instance, Item, Status, Storage,
};

use super::get_cached_window_activity;
use super::status_bar::{
    get_server_option, pid_is_running, set_server_option, unset_server_option, unset_session_option,
};

const NOTIFICATION_MONITOR_PID_OPTION_PREFIX: &str = "@aoe_notification_monitor_pid";
const NOTIFICATION_MONITOR_MTIME_OPTION_PREFIX: &str = "@aoe_notification_monitor_mtime";
const NOTIFICATION_OPTION: &str = "@aoe_waiting";

fn monitor_pid_option(profile: &str) -> String {
    if profile == "default" {
        NOTIFICATION_MONITOR_PID_OPTION_PREFIX.to_string()
    } else {
        format!("{NOTIFICATION_MONITOR_PID_OPTION_PREFIX}_{profile}")
    }
}

fn monitor_mtime_option(profile: &str) -> String {
    if profile == "default" {
        NOTIFICATION_MONITOR_MTIME_OPTION_PREFIX.to_string()
    } else {
        format!("{NOTIFICATION_MONITOR_MTIME_OPTION_PREFIX}_{profile}")
    }
}

fn current_exe_mtime() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let metadata = fs::metadata(exe).ok()?;
    let mtime = metadata.modified().ok()?;
    let duration = mtime.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(duration.as_secs().to_string())
}
const ACK_SIGNAL_FILE_NAME: &str = "ack-signal";
const FULL_CHECK_INTERVAL: Duration = Duration::from_secs(10);
const POLL_INTERVAL_RUNNING: Duration = Duration::from_secs(1);
const POLL_INTERVAL_WAITING: Duration = Duration::from_secs(2);
const POLL_INTERVAL_IDLE: Duration = Duration::from_secs(3);
const MAX_RETRY: Duration = Duration::from_secs(30);

/// Minimum interval between durable-store reconcile passes driven from this
/// long-lived monitor process. Mirrors the TUI status poller's interval. This
/// driver keeps `agent_slot` advancing while the TUI is attached to a session
/// (the poller is blocked on `tmux attach-session` and stops ticking), so the
/// resume/recovery data source does not go stale during normal attached use.
const RECONCILE_INTERVAL: Duration = Duration::from_millis(750);

#[derive(Debug, Clone)]
struct MonitorSessionState {
    title_sample: Option<titles::Sample>,
    last_window_activity: i64,
    last_full_check: Instant,
    primary_id: Option<String>,
    primary_status: Status,
    extra_statuses: HashMap<String, Status>,
    spike_start: Option<Instant>,
    pre_spike_status: Option<Status>,
    acknowledged: bool,
}

impl MonitorSessionState {
    fn new(now: Instant) -> Self {
        Self {
            title_sample: None,
            last_window_activity: i64::MIN,
            last_full_check: now - FULL_CHECK_INTERVAL,
            primary_id: None,
            primary_status: Status::Idle,
            extra_statuses: HashMap::new(),
            spike_start: None,
            pre_spike_status: None,
            acknowledged: false,
        }
    }

    fn clear_spike_state(&mut self) {
        self.spike_start = None;
        self.pre_spike_status = None;
    }

    fn apply_acknowledged_mapping(&self, status: Status) -> Status {
        if status == Status::Waiting && self.acknowledged {
            Status::Idle
        } else {
            status
        }
    }

    fn apply_spike_detection(
        &mut self,
        detected: Status,
        previous_status: Status,
        now: Instant,
    ) -> Status {
        if detected != Status::Running {
            self.clear_spike_state();
            return detected;
        }

        if previous_status == Status::Running {
            self.clear_spike_state();
            return Status::Running;
        }

        if self.spike_start.is_some() {
            self.clear_spike_state();
            return Status::Running;
        }

        self.spike_start = Some(now);
        self.pre_spike_status = Some(previous_status);
        previous_status
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NotificationEntry {
    session_name: String,
    title: String,
    status: Status,
    real_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionOptionUpdate {
    session_name: String,
    option: String,
    value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActivityGateDecision {
    activity_changed: bool,
    skip_capture: bool,
}

fn status_icon(status: Status) -> &'static str {
    match status {
        Status::Waiting => "\u{25d0}",
        Status::Running => "\u{25cf}",
        Status::Idle => "\u{25cb}",
        _ => "",
    }
}

fn current_home_sort_order() -> SortOrder {
    load_config()
        .ok()
        .flatten()
        .and_then(|config| config.app_state.sort_order)
        .unwrap_or_default()
}

fn ordered_existing_notification_entries(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
    existing_sessions: &HashSet<String>,
) -> Vec<NotificationEntry> {
    let expanded_groups = expanded_groups(groups);
    let group_tree = GroupTree::new_with_groups(instances, &expanded_groups);
    let instances_by_id: HashMap<&str, &Instance> = instances
        .iter()
        .map(|instance| (instance.id.as_str(), instance))
        .collect();

    flatten_tree(&group_tree, instances, sort_order)
        .into_iter()
        .filter_map(|item| match item {
            Item::Session { id, .. } => Some(id),
            Item::Group { .. } | Item::ProfileHeader { .. } => None,
        })
        .enumerate()
        .filter_map(|(index, id)| {
            let instance = instances_by_id.get(id.as_str()).copied()?;
            let session_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
            existing_sessions
                .contains(&session_name)
                .then_some((index + 1, session_name, instance))
        })
        .map(|(real_index, session_name, instance)| NotificationEntry {
            session_name,
            title: instance.title.clone(),
            status: instance.status,
            real_index,
        })
        .collect()
}

fn collapsed_group_paths(groups: &[Group]) -> HashSet<&str> {
    groups
        .iter()
        .filter(|group| group.collapsed)
        .map(|group| group.path.as_str())
        .collect()
}

fn is_in_collapsed_group(group_path: &str, collapsed_paths: &HashSet<&str>) -> bool {
    if group_path.is_empty() {
        return false;
    }

    let mut current = Some(group_path);
    while let Some(path) = current {
        if collapsed_paths.contains(path) {
            return true;
        }
        current = path.rsplit_once('/').map(|(parent, _)| parent);
    }

    false
}

fn should_notify_for_instance(
    instance: &Instance,
    effective_status: Status,
    collapsed_paths: &HashSet<&str>,
) -> bool {
    match effective_status {
        Status::Waiting => true,
        Status::Running | Status::Idle => {
            !is_in_collapsed_group(&instance.group_path, collapsed_paths)
        }
        _ => false,
    }
}

fn build_notification_entries(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
    existing_sessions: &HashSet<String>,
) -> Vec<NotificationEntry> {
    let collapsed_paths = collapsed_group_paths(groups);
    let visible_ids: HashSet<&str> = instances
        .iter()
        .filter(|instance| should_notify_for_instance(instance, instance.status, &collapsed_paths))
        .map(|instance| instance.id.as_str())
        .collect();

    let session_to_instance: HashMap<String, &Instance> = instances
        .iter()
        .map(|instance| {
            let name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
            (name, instance)
        })
        .collect();

    ordered_existing_notification_entries(instances, groups, sort_order, existing_sessions)
        .into_iter()
        .filter(|entry| {
            session_to_instance
                .get(&entry.session_name)
                .is_some_and(|instance| visible_ids.contains(instance.id.as_str()))
        })
        .collect()
}

fn visible_notification_entries<'a>(
    entries: &'a [NotificationEntry],
    current_session: &str,
) -> Vec<&'a NotificationEntry> {
    entries
        .iter()
        .filter(|entry| entry.session_name != current_session)
        .collect()
}

/// Returns a tmux color code matching the TUI theme for a given session status.
fn status_tmux_color(status: Status) -> &'static str {
    match status {
        Status::Waiting => "#[fg=#fbbf24]", // amber - theme.waiting
        Status::Running => "#[fg=#22c55e]", // green - theme.running
        Status::Idle => "#[fg=#64748b]",    // slate - theme.idle
        _ => "#[fg=#64748b]",               // slate - fallback
    }
}

fn format_notification_text(entries: &[NotificationEntry], current_session: &str) -> String {
    visible_notification_entries(entries, current_session)
        .into_iter()
        .map(|entry| {
            format!(
                "{}[{}] {} {}",
                status_tmux_color(entry.status),
                entry.real_index,
                status_icon(entry.status),
                entry.title
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_notification_session_updates(
    session_names: &[String],
    entries: &[NotificationEntry],
) -> Vec<SessionOptionUpdate> {
    let mut updates = Vec::new();

    for session_name in session_names {
        let notification_text = format_notification_text(entries, session_name);

        updates.push(SessionOptionUpdate {
            session_name: session_name.clone(),
            option: NOTIFICATION_OPTION.to_string(),
            value: (!notification_text.is_empty()).then_some(notification_text),
        });
    }

    updates
}

#[derive(Default)]
struct WriteRetry {
    next_attempt: Option<Instant>,
    delay: Duration,
}

impl WriteRetry {
    fn apply(&mut self, updates: &[SessionOptionUpdate], now: Instant) {
        if self.next_attempt.is_some_and(|next| next > now) {
            return;
        }
        match write_options(updates, super::control::output) {
            Ok(()) => *self = Self::default(),
            Err(error) => {
                self.delay = (self.delay * 2).clamp(POLL_INTERVAL_RUNNING, MAX_RETRY);
                self.next_attempt = Some(now + self.delay);
                tracing::warn!(
                    "Notification write failed: {error:#}; retry in {:?}",
                    self.delay
                );
            }
        }
    }
}

fn write_options(
    updates: &[SessionOptionUpdate],
    mut output: impl FnMut(&[&str]) -> std::io::Result<std::process::Output>,
) -> Result<()> {
    let mut failures = Vec::new();
    for update in updates {
        let target = format!("={}:", update.session_name);
        let args: &[&str] = match &update.value {
            Some(value) => &["set-option", "-t", &target, &update.option, value],
            None => &["set-option", "-u", "-t", &target, &update.option],
        };
        match output(args) {
            Ok(result) if result.status.success() => {}
            Ok(result) => failures.push(format!(
                "{target} ({}): {}",
                result.status,
                String::from_utf8_lossy(&result.stderr).trim()
            )),
            Err(error) => failures.push(format!("{target}: {error}")),
        }
    }
    anyhow::ensure!(
        failures.is_empty(),
        "Writing notification options: {}",
        failures.join("; ")
    );
    Ok(())
}

struct ExtraPane {
    id: String,
    tool: &'static str,
    title_status: Option<Status>,
}

struct DetectionPlan {
    primary_hint: Option<Status>,
    extras: Option<Vec<ExtraPane>>,
    allow_capture: bool,
}

impl DetectionPlan {
    fn prepare(
        instance: &Instance,
        infos: Option<Vec<super::PaneInfo>>,
        primary_id: Option<&str>,
        allow_capture: bool,
        requests: &mut captures::CaptureRequests,
    ) -> Self {
        let primary_hint = match crate::hooks::read_hook_status_with_freshness(&instance.id) {
            Some(read) if read.fresh => Some(read.status),
            Some(read) => {
                tracing::debug!(
                    "Notification hook stale for {} (value={:?}, age={}s)",
                    instance.id,
                    read.status,
                    read.age.as_secs()
                );
                None
            }
            None => None,
        }
        .or_else(|| {
            let id = primary_id?;
            let primary = infos.as_ref()?.iter().find(|p| p.pane_id == id)?;
            super::status_detection::detect_status_from_title(&primary.pane_title)
        });
        if allow_capture && primary_hint.is_none() {
            if let Some(id) = primary_id {
                requests.insert(id);
            }
        }
        let extras = infos.map(|infos| {
            infos
                .into_iter()
                .filter(|p| Some(p.pane_id.as_str()) != primary_id)
                .filter_map(|info| {
                    let tool = super::status_detection::detect_agent_type_from_pane(&info)?;
                    if tool == "shell" {
                        return None;
                    }
                    let title_status =
                        super::status_detection::detect_status_from_title(&info.pane_title);
                    if allow_capture && title_status.is_none() {
                        requests.insert(&info.pane_id);
                    }
                    Some(ExtraPane {
                        id: info.pane_id,
                        tool,
                        title_status,
                    })
                })
                .collect()
        });
        Self {
            primary_hint,
            extras,
            allow_capture,
        }
    }
}

fn detect_live_status(
    instance: &Instance,
    state: &mut MonitorSessionState,
    plan: DetectionPlan,
    screens: &HashMap<String, String>,
    sampled_primary: Option<Status>,
    now: Instant,
) -> Status {
    if let Some(status) = plan.primary_hint {
        state.clear_spike_state();
        state.primary_status = status;
    } else if plan.allow_capture {
        let detected = sampled_primary.or_else(|| {
            let content = screens.get(state.primary_id.as_ref()?)?;
            Some(super::status_detection::detect_status_from_content(
                content,
                &instance.tool,
                None,
            ))
        });
        if let Some(status) = detected {
            state.primary_status = state.apply_spike_detection(status, state.primary_status, now);
        }
    }

    if let Some(extras) = plan.extras {
        detect_extra_pane_statuses_for_monitor(extras, state, screens, plan.allow_capture, now);
    }
    let aggregated = if state.extra_statuses.is_empty() {
        state.primary_status
    } else {
        let mut statuses = vec![state.primary_status];
        statuses.extend(state.extra_statuses.values().copied());
        super::status_detection::aggregate_pane_statuses(&statuses)
    };
    state.apply_acknowledged_mapping(aggregated)
}

fn detect_extra_pane_statuses_for_monitor(
    extras: Vec<ExtraPane>,
    state: &mut MonitorSessionState,
    screens: &HashMap<String, String>,
    allow_capture: bool,
    now: Instant,
) {
    let mut statuses = HashMap::new();
    for pane in extras {
        let detected = pane.title_status.or_else(|| {
            if !allow_capture {
                return None;
            }
            if pane.tool == "codex" {
                let sample = state
                    .title_sample
                    .as_ref()
                    .and_then(|s| s.status_for(&pane.id, now));
                if sample.is_some() {
                    return sample;
                }
            }
            let content = screens.get(&pane.id)?;
            Some(super::status_detection::detect_status_from_content(
                content, pane.tool, None,
            ))
        });
        let status = detected.or_else(|| state.extra_statuses.get(&pane.id).copied());
        statuses.insert(pane.id, status.unwrap_or(Status::Idle));
    }
    state.extra_statuses = statuses;
}

fn decide_activity_gate(
    hook_based: bool,
    current_activity: Option<i64>,
    state: &MonitorSessionState,
    now: Instant,
) -> ActivityGateDecision {
    let activity_changed = match current_activity {
        Some(_) if state.last_window_activity == i64::MIN => true,
        Some(current) => current != state.last_window_activity,
        None => false,
    };
    let full_check_due = now.duration_since(state.last_full_check) >= FULL_CHECK_INTERVAL;

    ActivityGateDecision {
        activity_changed,
        skip_capture: !hook_based
            && current_activity.is_some()
            && !activity_changed
            && !full_check_due
            && state.spike_start.is_none(),
    }
}

fn poll_interval_for_statuses(statuses: impl IntoIterator<Item = Status>) -> Duration {
    let mut saw_waiting = false;

    for status in statuses {
        if status == Status::Running {
            return POLL_INTERVAL_RUNNING;
        }
        if status == Status::Waiting {
            saw_waiting = true;
        }
    }

    if saw_waiting {
        POLL_INTERVAL_WAITING
    } else {
        POLL_INTERVAL_IDLE
    }
}

fn ack_signal_path() -> Result<PathBuf> {
    Ok(crate::session::get_app_dir()?.join(ACK_SIGNAL_FILE_NAME))
}

pub fn write_ack_signal(instance_id: &str) -> Result<()> {
    fs::write(ack_signal_path()?, instance_id)?;
    Ok(())
}

fn take_ack_signal() -> Option<String> {
    let path = ack_signal_path().ok()?;
    let ack = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(path);
    let ack = ack.trim().to_string();
    (!ack.is_empty()).then_some(ack)
}

fn update_notification_options(
    profile: &str,
    snapshot: &inventory::Snapshot,
    states: &mut HashMap<String, MonitorSessionState>,
    acknowledged_instance_id: Option<&str>,
    writes: &mut WriteRetry,
) -> Result<(Duration, Vec<String>)> {
    let now = Instant::now();
    let storage = Storage::new(profile)?;
    let (mut instances, groups) = storage.load_with_groups()?;
    let panes = &snapshot.active;
    let existing_sessions: HashSet<String> = panes.keys().cloned().collect();
    let active_ids: HashSet<String> = instances
        .iter()
        .map(|instance| instance.id.clone())
        .collect();

    states.retain(|instance_id, _| active_ids.contains(instance_id));

    if let Some(instance_id) = acknowledged_instance_id {
        states
            .entry(instance_id.to_string())
            .or_insert_with(|| MonitorSessionState::new(now))
            .acknowledged = true;
    }

    let mut requests = captures::CaptureRequests::default();
    let mut pending = Vec::new();
    for instance in &mut instances {
        let session_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
        let Some(pane) = panes.get(&session_name) else {
            continue;
        };

        let hook_based = crate::agents::get_agent(&instance.tool)
            .is_some_and(|agent| agent.hook_config.is_some());
        let current_activity = get_cached_window_activity(&session_name);
        let state = states
            .entry(instance.id.clone())
            .or_insert_with(|| MonitorSessionState::new(now));
        let infos = super::get_all_cached_pane_infos(&session_name);
        let owns = |id: &str| {
            infos
                .as_ref()
                .is_some_and(|panes| panes.iter().any(|p| p.pane_id == id))
        };
        let primary_id = pane
            .as_ref()
            .map(titles::ActivePane::primary_id)
            .filter(|id| owns(id));
        let pane = pane.as_ref().filter(|pane| owns(pane.id()));
        if let Some(pane) = pane {
            pane.request_title_capture(instance, state.title_sample.as_ref(), now, &mut requests);
        }
        let decision = decide_activity_gate(hook_based, current_activity, state, now);
        let switched = primary_id != state.primary_id.as_deref();
        let allow_capture = !decision.skip_capture || switched;
        if switched {
            state.clear_spike_state();
            if pane.is_some() && infos.is_some() {
                state.primary_status = Status::Idle;
            }
            state.primary_id = primary_id.map(str::to_owned);
        }
        if decision.activity_changed {
            state.acknowledged = false;
        }
        if let Some(activity) = current_activity {
            state.last_window_activity = activity;
        }
        if !hook_based && allow_capture {
            state.last_full_check = now;
        }

        let plan =
            DetectionPlan::prepare(instance, infos, primary_id, allow_capture, &mut requests);
        pending.push((instance, session_name, pane, plan));
    }

    let screens = requests.capture();
    for (instance, session_name, pane, plan) in pending {
        let state = states.get_mut(&instance.id).expect("planned monitor state");
        if let Some(pane) = pane {
            if let Err(error) = pane.refresh_title(instance, &mut state.title_sample, now, &screens)
            {
                tracing::warn!("Title refresh for {} failed: {error:#}", session_name);
            }
        }
        let sampled_primary =
            pane.and_then(|pane| pane.fresh_primary_status(state.title_sample.as_ref(), now));
        instance.status = detect_live_status(instance, state, plan, &screens, sampled_primary, now);
    }

    let sort_order = current_home_sort_order();
    let entries = build_notification_entries(&instances, &groups, sort_order, &existing_sessions);
    let profile_sessions: Vec<String> = instances
        .iter()
        .map(|inst| crate::tmux::Session::generate_name(&inst.id, &inst.title))
        .filter(|name| existing_sessions.contains(name))
        .collect();
    let updates: Vec<_> = build_notification_session_updates(&profile_sessions, &entries)
        .into_iter()
        .filter(|update| {
            match inventory::notification_differs(
                &update.session_name,
                update.value.as_deref().unwrap_or_default(),
            ) {
                Ok(changed) => changed,
                Err(error) => {
                    tracing::warn!(
                        "Skipping notification for {}: {error:#}",
                        update.session_name
                    );
                    false
                }
            }
        })
        .collect();
    writes.apply(&updates, now);

    let interval = poll_interval_for_statuses(instances.iter().map(|instance| instance.status));
    Ok((interval, profile_sessions))
}

fn clear_notification_options(session_names: &[String]) {
    for session_name in session_names {
        let _ = unset_session_option(&format!("={session_name}:"), NOTIFICATION_OPTION);
    }
}

pub fn ensure_notification_monitor(profile: &str) -> Result<()> {
    let pid_option = monitor_pid_option(profile);
    let mtime_option = monitor_mtime_option(profile);

    if let Some(existing_pid) = get_server_option(&pid_option) {
        if pid_is_running(&existing_pid) {
            let stored_mtime = get_server_option(&mtime_option);
            let current_mtime = current_exe_mtime();
            if stored_mtime == current_mtime {
                legacy_owner::retire(profile, &pid_option, &existing_pid);
                return Ok(());
            }
            tracing::debug!(
                "Binary mtime changed for profile {}, restarting notification monitor",
                profile
            );
        }
    }

    let current_exe = std::env::current_exe()?;
    let mut child = Command::new(&current_exe);
    child
        .args(["tmux", "monitor-notifications", "--profile", profile])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut monitor = child.spawn()?;
    let monitor_pid = monitor.id();
    thread::Builder::new()
        .name("aoe-monitor-reaper".into())
        .spawn(move || {
            if let Err(error) = monitor.wait() {
                tracing::warn!("Reaping notification monitor {monitor_pid} failed: {error}");
            }
        })?;
    // Publishing the new owner lets the previous monitor finish its turn and exit.
    set_server_option(&pid_option, &monitor_pid.to_string())?;
    if let Some(mtime) = current_exe_mtime() {
        set_server_option(&mtime_option, &mtime)?;
    }
    legacy_owner::retire(profile, &pid_option, &monitor_pid.to_string());
    Ok(())
}

pub fn run_notification_monitor(profile: &str) -> Result<()> {
    super::control::with_monitor(profile, || monitor_loop(profile))
}

fn monitor_loop(profile: &str) -> Result<()> {
    let pid_option = monitor_pid_option(profile);
    let pid = std::process::id().to_string();
    let startup_deadline = Instant::now() + Duration::from_secs(2);
    let mut states = HashMap::<String, MonitorSessionState>::new();
    let mut last_profile_sessions = Vec::new();
    // Start in the past so the first loop iteration reconciles immediately.
    let mut last_reconcile = Instant::now() - RECONCILE_INTERVAL;

    let mut retry = POLL_INTERVAL_RUNNING;
    let mut writes = WriteRetry::default();
    let mut server = None;
    loop {
        let snapshot = match super::control::ensure_connected(&pid_option, &pid)
            .and_then(|connected| connected.then(|| inventory::read(&pid_option)).transpose())
        {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) if server.is_none() && Instant::now() <= startup_deadline => {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            Ok(None) => return Ok(()),
            Err(error) => {
                if let Some((pid, _)) = server {
                    if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None)
                        == Err(nix::errno::Errno::ESRCH)
                    {
                        return Ok(());
                    }
                } else if Instant::now() > startup_deadline {
                    return Err(error);
                }
                tracing::warn!(
                    "Notification monitor snapshot failed: {error:#}; retry in {retry:?}"
                );
                thread::sleep(retry);
                retry = (retry * 2).min(MAX_RETRY);
                continue;
            }
        };
        if server.is_some_and(|previous| previous != snapshot.server) {
            return Ok(());
        }
        if snapshot.owner != pid {
            if server.is_none() && Instant::now() <= startup_deadline {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            break;
        }
        if snapshot.release_unattached_server || snapshot.active.is_empty() {
            break;
        }
        server = Some(snapshot.server);

        let ack_signal = take_ack_signal();
        let notification_result = update_notification_options(
            profile,
            &snapshot,
            &mut states,
            ack_signal.as_deref(),
            &mut writes,
        );
        // Reconcile durable agent_slot records independently of the TUI status
        // poller. The poller only ticks while the user is on the home view; this
        // long-lived process keeps reconciling while attached to a session.
        // Best-effort and idempotent (the home-view driver may also run it).
        if last_reconcile.elapsed() >= RECONCILE_INTERVAL {
            crate::db::reconcile::reconcile_profile(profile);
            last_reconcile = Instant::now();
        }

        let (poll_interval, profile_sessions) = match notification_result {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!("Notification refresh failed: {err:#}; retry in {retry:?}");
                thread::sleep(retry);
                retry = (retry * 2).min(MAX_RETRY);
                continue;
            }
        };
        retry = POLL_INTERVAL_RUNNING;
        if !profile_sessions.is_empty() {
            last_profile_sessions = profile_sessions;
        }

        thread::sleep(poll_interval);
    }

    if get_server_option(&pid_option).as_deref() == Some(pid.as_str()) {
        clear_notification_options(&last_profile_sessions);
        let _ = unset_server_option(&pid_option);
    }

    Ok(())
}

pub fn clear_notification_option_for_current_session() {
    let Some(session_name) = crate::tmux::get_current_session_name() else {
        return;
    };
    if !session_name.starts_with(crate::tmux::SESSION_PREFIX) {
        return;
    }
    let _ = unset_session_option(&session_name, NOTIFICATION_OPTION);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::config::SortOrder;
    use crate::session::Status;
    use serial_test::serial;
    use tempfile::TempDir;

    fn setup_test_home(temp: &TempDir) -> crate::session::TestHomeGuard {
        crate::session::scoped_test_home(temp.path())
    }

    #[test]
    fn test_format_notification_text_with_multiple_sessions() {
        let entries = vec![
            NotificationEntry {
                session_name: "aoe_alpha_1".to_string(),
                title: "alpha".to_string(),
                status: Status::Waiting,
                real_index: 1,
            },
            NotificationEntry {
                session_name: "aoe_beta_2".to_string(),
                title: "beta".to_string(),
                status: Status::Idle,
                real_index: 2,
            },
            NotificationEntry {
                session_name: "aoe_gamma_3".to_string(),
                title: "gamma".to_string(),
                status: Status::Waiting,
                real_index: 5,
            },
        ];

        assert_eq!(
            format_notification_text(&entries, "aoe_missing"),
            "#[fg=#fbbf24][1] \u{25d0} alpha #[fg=#64748b][2] \u{25cb} beta #[fg=#fbbf24][5] \u{25d0} gamma"
        );
    }

    #[test]
    fn test_format_notification_text_excludes_self_and_keeps_real_indices() {
        let entries = vec![
            NotificationEntry {
                session_name: "aoe_alpha_1".to_string(),
                title: "alpha".to_string(),
                status: Status::Waiting,
                real_index: 2,
            },
            NotificationEntry {
                session_name: "aoe_beta_2".to_string(),
                title: "beta".to_string(),
                status: Status::Idle,
                real_index: 5,
            },
        ];

        assert_eq!(
            format_notification_text(&entries, "aoe_alpha_1"),
            "#[fg=#64748b][5] \u{25cb} beta"
        );
    }

    #[test]
    fn test_format_notification_text_handles_empty_list() {
        assert_eq!(format_notification_text(&[], "aoe_alpha_1"), "");
    }

    #[test]
    fn test_build_notification_entries_filters_idle_sessions_in_collapsed_groups() {
        let mut waiting = Instance::new("waiting", "/tmp/waiting");
        waiting.status = Status::Waiting;
        let mut idle_hidden = Instance::new("idle-hidden", "/tmp/idle-hidden");
        idle_hidden.group_path = "work".to_string();
        let mut idle_visible = Instance::new("idle-visible", "/tmp/idle-visible");
        idle_visible.group_path = "personal".to_string();

        let waiting_name = crate::tmux::Session::generate_name(&waiting.id, &waiting.title);
        let idle_hidden_name =
            crate::tmux::Session::generate_name(&idle_hidden.id, &idle_hidden.title);
        let idle_visible_name =
            crate::tmux::Session::generate_name(&idle_visible.id, &idle_visible.title);

        let instances = vec![waiting.clone(), idle_hidden.clone(), idle_visible.clone()];
        let groups = vec![
            Group {
                name: "work".to_string(),
                path: "work".to_string(),
                collapsed: true,
                default_directory: None,
                children: Vec::new(),
            },
            Group {
                name: "personal".to_string(),
                path: "personal".to_string(),
                collapsed: false,
                default_directory: None,
                children: Vec::new(),
            },
        ];
        let existing_sessions = HashSet::from([
            waiting_name.clone(),
            idle_hidden_name.clone(),
            idle_visible_name.clone(),
        ]);

        let entries =
            build_notification_entries(&instances, &groups, SortOrder::AZ, &existing_sessions);
        let session_names: Vec<_> = entries
            .iter()
            .map(|entry| entry.session_name.clone())
            .collect();

        assert!(session_names.contains(&waiting_name));
        assert!(session_names.contains(&idle_visible_name));
        assert!(!session_names.contains(&idle_hidden_name));

        let entries_by_session: HashMap<_, _> = entries
            .into_iter()
            .map(|entry| (entry.session_name.clone(), entry))
            .collect();
        assert_eq!(entries_by_session[&waiting_name].real_index, 1);
        assert_eq!(entries_by_session[&idle_visible_name].real_index, 2);
    }

    #[test]
    fn test_waiting_sessions_ignore_collapsed_parent_group() {
        let mut waiting = Instance::new("waiting", "/tmp/waiting");
        waiting.status = Status::Waiting;
        waiting.group_path = "work/nested".to_string();

        let mut idle = Instance::new("idle", "/tmp/idle");
        idle.group_path = "work/nested".to_string();

        let waiting_name = crate::tmux::Session::generate_name(&waiting.id, &waiting.title);
        let idle_name = crate::tmux::Session::generate_name(&idle.id, &idle.title);

        let instances = vec![waiting.clone(), idle.clone()];
        let groups = vec![
            Group {
                name: "work".to_string(),
                path: "work".to_string(),
                collapsed: true,
                default_directory: None,
                children: Vec::new(),
            },
            Group {
                name: "nested".to_string(),
                path: "work/nested".to_string(),
                collapsed: false,
                default_directory: None,
                children: Vec::new(),
            },
        ];
        let existing_sessions = HashSet::from([waiting_name.clone(), idle_name.clone()]);

        let entries =
            build_notification_entries(&instances, &groups, SortOrder::AZ, &existing_sessions);
        let session_names: Vec<_> = entries
            .into_iter()
            .map(|entry| entry.session_name)
            .collect();

        assert!(session_names.contains(&waiting_name));
        assert!(!session_names.contains(&idle_name));
    }

    #[test]
    fn test_ordered_existing_notification_entries_use_expanded_tree_indices() {
        let alpha = Instance::new("alpha", "/tmp/alpha");

        let mut beta = Instance::new("beta", "/tmp/beta");
        beta.group_path = "work".to_string();

        let mut gamma = Instance::new("gamma", "/tmp/gamma");
        gamma.group_path = "personal".to_string();

        let alpha_name = crate::tmux::Session::generate_name(&alpha.id, &alpha.title);
        let beta_name = crate::tmux::Session::generate_name(&beta.id, &beta.title);
        let gamma_name = crate::tmux::Session::generate_name(&gamma.id, &gamma.title);

        let groups = vec![
            Group {
                name: "work".to_string(),
                path: "work".to_string(),
                collapsed: true,
                default_directory: None,
                children: Vec::new(),
            },
            Group {
                name: "personal".to_string(),
                path: "personal".to_string(),
                collapsed: false,
                default_directory: None,
                children: Vec::new(),
            },
        ];
        let existing_sessions =
            HashSet::from([alpha_name.clone(), beta_name.clone(), gamma_name.clone()]);

        let entries = ordered_existing_notification_entries(
            &[alpha.clone(), beta.clone(), gamma.clone()],
            &groups,
            SortOrder::AZ,
            &existing_sessions,
        );

        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.session_name.as_str(), entry.real_index))
                .collect::<Vec<_>>(),
            vec![
                (alpha_name.as_str(), 1),
                (gamma_name.as_str(), 2),
                (beta_name.as_str(), 3),
            ]
        );
    }

    #[test]
    fn test_is_in_collapsed_group_empty_path() {
        let collapsed = HashSet::from(["work"]);
        assert!(!is_in_collapsed_group("", &collapsed));
    }

    #[test]
    fn test_is_in_collapsed_group_direct_match() {
        let collapsed = HashSet::from(["work"]);
        assert!(is_in_collapsed_group("work", &collapsed));
    }

    #[test]
    fn test_is_in_collapsed_group_ancestor_match() {
        let collapsed = HashSet::from(["work"]);
        assert!(is_in_collapsed_group("work/nested/deep", &collapsed));
    }

    #[test]
    fn test_is_in_collapsed_group_no_match() {
        let collapsed = HashSet::from(["work"]);
        assert!(!is_in_collapsed_group("personal", &collapsed));
    }

    #[test]
    fn test_monitor_session_state_activity_reset_and_acknowledged_mapping() {
        let now = Instant::now();
        let mut state = MonitorSessionState::new(now);

        assert_eq!(
            state.apply_acknowledged_mapping(Status::Waiting),
            Status::Waiting
        );

        state.acknowledged = true;
        assert_eq!(
            state.apply_acknowledged_mapping(Status::Waiting),
            Status::Idle
        );
        assert_eq!(
            state.apply_acknowledged_mapping(Status::Running),
            Status::Running
        );

        state.last_window_activity = 42;
        let decision = decide_activity_gate(false, Some(43), &state, now);
        assert!(decision.activity_changed);
    }

    #[test]
    fn test_monitor_spike_detection_rejects_transient_running() {
        let now = Instant::now();
        let mut state = MonitorSessionState::new(now);

        let first = state.apply_spike_detection(Status::Running, Status::Idle, now);
        assert_eq!(first, Status::Idle);
        assert!(state.spike_start.is_some());
        assert_eq!(state.pre_spike_status, Some(Status::Idle));

        let second = state.apply_spike_detection(Status::Idle, first, now);
        assert_eq!(second, Status::Idle);
        assert!(state.spike_start.is_none());
        assert!(state.pre_spike_status.is_none());
    }

    #[test]
    fn test_monitor_spike_detection_confirms_persistent_running() {
        let now = Instant::now();
        let mut state = MonitorSessionState::new(now);

        let first = state.apply_spike_detection(Status::Running, Status::Waiting, now);
        assert_eq!(first, Status::Waiting);

        let second = state.apply_spike_detection(Status::Running, first, now);
        assert_eq!(second, Status::Running);
        assert!(state.spike_start.is_none());
        assert!(state.pre_spike_status.is_none());
    }

    #[test]
    fn test_poll_interval_for_statuses_prefers_running_then_waiting() {
        assert_eq!(
            poll_interval_for_statuses([Status::Idle, Status::Running, Status::Waiting]),
            POLL_INTERVAL_RUNNING
        );
        assert_eq!(
            poll_interval_for_statuses([Status::Idle, Status::Waiting]),
            POLL_INTERVAL_WAITING
        );
        assert_eq!(
            poll_interval_for_statuses([Status::Idle, Status::Error]),
            POLL_INTERVAL_IDLE
        );
    }

    #[test]
    #[serial]
    fn test_write_ack_signal_round_trips_through_take_ack_signal() {
        let temp = TempDir::new().unwrap();
        let _home = setup_test_home(&temp);

        write_ack_signal("abc123").unwrap();
        assert_eq!(take_ack_signal(), Some("abc123".to_string()));
        assert_eq!(take_ack_signal(), None);
    }

    #[test]
    fn test_notification_monitor_ignores_stale_hook_file() {
        // Confirms the notification monitor uses the freshness-aware hook
        // reader. If the hook file is stale, `read_hook_status_with_freshness`
        // reports `fresh=false` and the monitor's `detect_live_status`
        // short-circuit is skipped; the monitor must then fall through to
        // content detection (or return Idle in the absence of a live tmux
        // session, matching the TUI poller).
        let id = "test_notification_monitor_stale_hook";
        let dir = std::path::Path::new("/tmp/aoe-hooks").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("status");
        std::fs::write(&path, "running").unwrap();

        use chrono::{Local, TimeZone};
        let target = Local::now() - chrono::Duration::seconds(120);
        let stamp = Local
            .timestamp_opt(target.timestamp(), 0)
            .single()
            .unwrap()
            .format("%Y%m%d%H%M.%S")
            .to_string();
        let status = std::process::Command::new("touch")
            .args(["-t", &stamp, path.to_str().unwrap()])
            .status()
            .expect("touch should run");
        assert!(status.success());

        let read = crate::hooks::read_hook_status_with_freshness(id).expect("file present");
        assert!(
            !read.fresh,
            "stale hook (mtime 120s ago) must not be treated as fresh"
        );
        assert_eq!(read.status, Status::Running);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_options_keeps_values_literal_and_continues_after_a_missing_target() {
        use std::os::unix::process::ExitStatusExt;
        let value = "waiting; #{} '$title'\nnext";
        let updates = [
            SessionOptionUpdate {
                session_name: "aoe_alpha".into(),
                option: NOTIFICATION_OPTION.into(),
                value: Some(value.into()),
            },
            SessionOptionUpdate {
                session_name: "aoe_beta".into(),
                option: NOTIFICATION_OPTION.into(),
                value: None,
            },
        ];
        let mut commands = Vec::new();
        let error = write_options(&updates, |args| {
            commands.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            let missing = commands.len() == 1;
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(if missing { 1 << 8 } else { 0 }),
                stdout: Vec::new(),
                stderr: if missing {
                    b"missing target".to_vec()
                } else {
                    Vec::new()
                },
            })
        })
        .unwrap_err();
        assert!(error.to_string().contains("missing target"));
        assert_eq!(
            commands,
            [
                [
                    "set-option",
                    "-t",
                    "=aoe_alpha:",
                    NOTIFICATION_OPTION,
                    value
                ],
                ["set-option", "-u", "-t", "=aoe_beta:", NOTIFICATION_OPTION],
            ]
        );
    }
}
