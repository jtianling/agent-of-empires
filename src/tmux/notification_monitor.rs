//! Background notification monitor for tmux status bar.
//!
//! Polls session statuses and updates per-session tmux user options so the
//! status bar can display Waiting/Running/Idle sessions.

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

use super::status_bar::{
    get_server_option, list_aoe_sessions, pid_is_running, set_server_option, set_session_option,
    unset_server_option, unset_session_option,
};
use super::{
    get_cached_pane_info, get_cached_window_activity, refresh_pane_info_cache,
    refresh_session_cache,
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

#[derive(Debug, Clone)]
struct MonitorSessionState {
    last_window_activity: i64,
    last_full_check: Instant,
    last_status: Status,
    spike_start: Option<Instant>,
    pre_spike_status: Option<Status>,
    acknowledged: bool,
}

impl MonitorSessionState {
    fn new(now: Instant) -> Self {
        Self {
            last_window_activity: i64::MIN,
            last_full_check: now - FULL_CHECK_INTERVAL,
            last_status: Status::Idle,
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
            Item::Group { .. } => None,
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

fn format_notification_text(entries: &[NotificationEntry], current_session: &str) -> String {
    visible_notification_entries(entries, current_session)
        .into_iter()
        .map(|entry| {
            format!(
                "[{}] {} {}",
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

fn build_batched_session_option_args(updates: &[SessionOptionUpdate]) -> Vec<String> {
    let mut args = Vec::new();

    for (index, update) in updates.iter().enumerate() {
        if index > 0 {
            args.push(";".to_string());
        }

        args.push("set-option".to_string());
        args.push("-t".to_string());
        args.push(update.session_name.clone());

        if update.value.is_none() {
            args.push("-u".to_string());
        }

        args.push(update.option.clone());

        if let Some(value) = &update.value {
            args.push(value.clone());
        }
    }

    args
}

fn apply_session_option_updates(updates: &[SessionOptionUpdate]) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }

    let args = build_batched_session_option_args(updates);
    let output = Command::new("tmux").args(&args).output()?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    tracing::debug!(
        "Batched notification option update failed, falling back to individual writes: {}",
        stderr
    );

    for update in updates {
        if let Some(value) = &update.value {
            set_session_option(&update.session_name, &update.option, value)?;
        } else {
            unset_session_option(&update.session_name, &update.option)?;
        }
    }

    Ok(())
}

fn detect_live_status(
    instance: &Instance,
    state: &mut MonitorSessionState,
    allow_capture: bool,
    now: Instant,
) -> Status {
    let previous_status = state.last_status;
    let mut primary_status: Option<Status> = None;

    if let Some(hook_status) = crate::hooks::read_hook_status(&instance.id) {
        state.clear_spike_state();
        primary_status = Some(hook_status);
    }

    let session_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);

    if primary_status.is_none() {
        if let Some(status) = get_cached_pane_info(&session_name)
            .and_then(|info| super::status_detection::detect_status_from_title(&info.pane_title))
        {
            state.clear_spike_state();
            primary_status = Some(status);
        }
    }

    if primary_status.is_none() {
        let mut detected = if allow_capture {
            match instance.tmux_session() {
                Ok(session) => match session.capture_pane_cached(50) {
                    Ok(content) => super::status_detection::detect_status_from_content(
                        &content,
                        &instance.tool,
                        None,
                    ),
                    Err(_) => previous_status,
                },
                Err(_) => previous_status,
            }
        } else {
            previous_status
        };

        if allow_capture {
            detected = state.apply_spike_detection(detected, previous_status, now);
        }

        primary_status = Some(detected);
    }

    let primary_status = primary_status.unwrap_or(Status::Idle);

    // Detect status for extra (user-split) panes and aggregate
    let extra_statuses = detect_extra_pane_statuses_for_monitor(&session_name, allow_capture);
    let aggregated = if extra_statuses.is_empty() {
        primary_status
    } else {
        let mut all = vec![primary_status];
        all.extend(extra_statuses);
        super::status_detection::aggregate_pane_statuses(&all)
    };

    let result = state.apply_acknowledged_mapping(aggregated);
    state.last_status = result;
    result
}

fn detect_extra_pane_statuses_for_monitor(session_name: &str, allow_capture: bool) -> Vec<Status> {
    let all_panes = match super::get_all_cached_pane_infos(session_name) {
        Some(panes) if panes.len() > 1 => panes,
        _ => return Vec::new(),
    };

    let mut statuses = Vec::new();

    for pane_info in all_panes.iter().skip(1) {
        let agent_type = match super::status_detection::detect_agent_type_from_pane(pane_info) {
            Some("shell") | None => continue,
            Some(agent) => agent,
        };

        if let Some(status) =
            super::status_detection::detect_status_from_title(&pane_info.pane_title)
        {
            statuses.push(status);
            continue;
        }

        if allow_capture {
            if let Ok(content) = crate::tmux::Session::capture_pane_by_id(&pane_info.pane_id, 50) {
                let status =
                    super::status_detection::detect_status_from_content(&content, agent_type, None);
                statuses.push(status);
                continue;
            }
        }

        statuses.push(Status::Idle);
    }

    statuses
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
    session_names: &[String],
    states: &mut HashMap<String, MonitorSessionState>,
    acknowledged_instance_id: Option<&str>,
) -> Result<(Duration, Vec<String>)> {
    let now = Instant::now();
    let storage = Storage::new(profile)?;
    let (mut instances, groups) = storage.load_with_groups()?;
    let existing_sessions: HashSet<String> = session_names.iter().cloned().collect();
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

    for instance in &mut instances {
        let session_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
        if !existing_sessions.contains(&session_name) {
            continue;
        }

        let hook_based = crate::agents::get_agent(&instance.tool)
            .is_some_and(|agent| agent.hook_config.is_some());
        let current_activity = get_cached_window_activity(&session_name);
        let state = states
            .entry(instance.id.clone())
            .or_insert_with(|| MonitorSessionState::new(now));
        let decision = decide_activity_gate(hook_based, current_activity, state, now);

        if decision.activity_changed {
            state.acknowledged = false;
        }
        if let Some(activity) = current_activity {
            state.last_window_activity = activity;
        }
        if !hook_based && !decision.skip_capture {
            state.last_full_check = now;
        }

        instance.status = detect_live_status(instance, state, !decision.skip_capture, now);
    }

    let sort_order = current_home_sort_order();
    let entries = build_notification_entries(&instances, &groups, sort_order, &existing_sessions);
    let profile_sessions: Vec<String> = instances
        .iter()
        .map(|inst| crate::tmux::Session::generate_name(&inst.id, &inst.title))
        .filter(|name| existing_sessions.contains(name))
        .collect();
    let updates = build_notification_session_updates(&profile_sessions, &entries);
    apply_session_option_updates(&updates)?;

    let interval = poll_interval_for_statuses(instances.iter().map(|instance| instance.status));
    Ok((interval, profile_sessions))
}

fn clear_notification_options(session_names: &[String]) {
    for session_name in session_names {
        let _ = unset_session_option(session_name, NOTIFICATION_OPTION);
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
                return Ok(());
            }
            tracing::debug!(
                "Binary mtime changed for profile {}, restarting notification monitor",
                profile
            );
            kill_process(&existing_pid);
        }
    }

    let current_exe = std::env::current_exe()?;
    let mut child = Command::new(&current_exe);
    child
        .args(["tmux", "monitor-notifications", "--profile", profile])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let monitor = child.spawn()?;
    set_server_option(&pid_option, &monitor.id().to_string())?;
    if let Some(mtime) = current_exe_mtime() {
        set_server_option(&mtime_option, &mtime)?;
    }
    Ok(())
}

fn kill_process(pid_str: &str) {
    if let Ok(pid) = pid_str.parse::<i32>() {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
}

pub fn run_notification_monitor(profile: &str) -> Result<()> {
    let pid_option = monitor_pid_option(profile);
    let pid = std::process::id().to_string();
    let startup_deadline = Instant::now() + Duration::from_secs(2);
    let mut states = HashMap::<String, MonitorSessionState>::new();
    let mut last_profile_sessions = Vec::new();

    loop {
        match get_server_option(&pid_option) {
            Some(owner) if owner == pid => {}
            Some(_) => break,
            None if Instant::now() <= startup_deadline => {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            None => break,
        }

        refresh_session_cache();
        refresh_pane_info_cache();

        let session_names = list_aoe_sessions()?;
        if session_names.is_empty() {
            break;
        }

        let ack_signal = take_ack_signal();
        let (poll_interval, profile_sessions) = match update_notification_options(
            profile,
            &session_names,
            &mut states,
            ack_signal.as_deref(),
        ) {
            Ok(result) => result,
            Err(err) => {
                tracing::debug!("Failed to refresh notification options: {}", err);
                (POLL_INTERVAL_WAITING, Vec::new())
            }
        };
        if !profile_sessions.is_empty() {
            last_profile_sessions = profile_sessions;
        }

        if get_server_option(&pid_option).as_deref() != Some(pid.as_str()) {
            break;
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

    fn setup_test_home(temp: &TempDir) {
        std::env::set_var("HOME", temp.path());
        #[cfg(target_os = "linux")]
        std::env::set_var("XDG_CONFIG_HOME", temp.path().join(".config"));
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
            "[1] \u{25d0} alpha [2] \u{25cb} beta [5] \u{25d0} gamma"
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
            "[5] \u{25cb} beta"
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
        setup_test_home(&temp);

        write_ack_signal("abc123").unwrap();
        assert_eq!(take_ack_signal(), Some("abc123".to_string()));
        assert_eq!(take_ack_signal(), None);
    }

    #[test]
    fn test_build_batched_session_option_args_supports_set_and_unset() {
        let args = build_batched_session_option_args(&[
            SessionOptionUpdate {
                session_name: "aoe_alpha".to_string(),
                option: "@aoe_waiting".to_string(),
                value: Some("[1] waiting".to_string()),
            },
            SessionOptionUpdate {
                session_name: "aoe_beta".to_string(),
                option: "@aoe_waiting".to_string(),
                value: None,
            },
        ]);

        assert_eq!(
            args,
            vec![
                "set-option",
                "-t",
                "aoe_alpha",
                "@aoe_waiting",
                "[1] waiting",
                ";",
                "set-option",
                "-t",
                "aoe_beta",
                "-u",
                "@aoe_waiting",
            ]
        );
    }
}
