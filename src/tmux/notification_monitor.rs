//! Background notification monitor for tmux status bar.
//!
//! Polls session statuses and updates per-session tmux user options so the
//! status bar can display Waiting/Idle sessions.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::session::config::{load_config, SortOrder};
use crate::session::{flatten_tree, Group, GroupTree, Instance, Item, Status, Storage};

use super::status_bar::{
    get_server_option, list_aoe_sessions, pid_is_running, set_server_option, set_session_option,
    unset_server_option, unset_session_option,
};
use super::{
    get_cached_pane_info, get_cached_window_activity, refresh_pane_info_cache,
    refresh_session_cache,
};

const NOTIFICATION_MONITOR_PID_OPTION: &str = "@aoe_notification_monitor_pid";
const NOTIFICATION_OPTION: &str = "@aoe_waiting";
const NOTIFICATION_HINT_OPTION: &str = "@aoe_notification_hint";
const NOTIFICATION_TRIGGER_KEY: &str = "N";
const NOTIFICATION_KEY_TABLE: &str = "aoe_notify";
const ACK_SIGNAL_FILE_NAME: &str = "ack-signal";
const FULL_CHECK_INTERVAL: Duration = Duration::from_secs(10);
const POLL_INTERVAL_RUNNING: Duration = Duration::from_secs(1);
const POLL_INTERVAL_WAITING: Duration = Duration::from_secs(2);
const POLL_INTERVAL_IDLE: Duration = Duration::from_secs(3);
const MAX_NOTIFICATION_BINDINGS: usize = 6;

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
    instance_id: String,
    session_name: String,
    title: String,
    status: Status,
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

fn expanded_groups(groups: &[Group]) -> Vec<Group> {
    groups
        .iter()
        .map(|group| Group {
            name: group.name.clone(),
            path: group.path.clone(),
            collapsed: false,
            default_directory: group.default_directory.clone(),
            children: Vec::new(),
        })
        .collect()
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
            Item::Session { id, .. } => instances_by_id.get(id.as_str()).copied(),
            Item::Group { .. } => None,
        })
        .filter_map(|instance| {
            let session_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
            existing_sessions
                .contains(&session_name)
                .then_some((session_name, instance))
        })
        .map(|(session_name, instance)| NotificationEntry {
            instance_id: instance.id.clone(),
            session_name,
            title: instance.title.clone(),
            status: instance.status,
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
        Status::Idle => !is_in_collapsed_group(&instance.group_path, collapsed_paths),
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
        .enumerate()
        .map(|(index, entry)| {
            format!(
                "[{}] {} {}",
                index + 1,
                status_icon(entry.status),
                entry.title
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn notification_option_name(prefix: &str, index: usize) -> String {
    format!("{prefix}_{index}")
}

fn notification_binding_hint(visible_entries: usize) -> Option<String> {
    (visible_entries > 0).then(|| {
        let upper = visible_entries.min(MAX_NOTIFICATION_BINDINGS);
        if upper == 1 {
            format!("Ctrl+b {} 1 notify", NOTIFICATION_TRIGGER_KEY)
        } else {
            format!("Ctrl+b {} 1..{} notify", NOTIFICATION_TRIGGER_KEY, upper)
        }
    })
}

fn build_notification_session_updates(
    session_names: &[String],
    entries: &[NotificationEntry],
) -> Vec<SessionOptionUpdate> {
    let mut updates = Vec::new();

    for session_name in session_names {
        let visible_entries = visible_notification_entries(entries, session_name);
        let notification_text = format_notification_text(entries, session_name);

        updates.push(SessionOptionUpdate {
            session_name: session_name.clone(),
            option: NOTIFICATION_OPTION.to_string(),
            value: (!notification_text.is_empty()).then_some(notification_text),
        });
        updates.push(SessionOptionUpdate {
            session_name: session_name.clone(),
            option: NOTIFICATION_HINT_OPTION.to_string(),
            value: notification_binding_hint(visible_entries.len()),
        });

        for index in 1..=MAX_NOTIFICATION_BINDINGS {
            let visible = visible_entries.get(index - 1).copied();
            updates.push(SessionOptionUpdate {
                session_name: session_name.clone(),
                option: notification_option_name("@aoe_notify_target", index),
                value: visible.map(|entry| entry.session_name.clone()),
            });
            updates.push(SessionOptionUpdate {
                session_name: session_name.clone(),
                option: notification_option_name("@aoe_notify_instance", index),
                value: visible.map(|entry| entry.instance_id.clone()),
            });
        }
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

    if let Some(hook_status) = crate::hooks::read_hook_status(&instance.id) {
        state.clear_spike_state();
        let status = state.apply_acknowledged_mapping(hook_status);
        state.last_status = status;
        return status;
    }

    let session_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
    if let Some(status) = get_cached_pane_info(&session_name)
        .and_then(|info| super::status_detection::detect_status_from_title(&info.pane_title))
    {
        state.clear_spike_state();
        state.last_status = status;
        return status;
    }

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
    detected = state.apply_acknowledged_mapping(detected);
    state.last_status = detected;
    detected
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

fn take_ack_signal() -> Option<String> {
    let path = ack_signal_path().ok()?;
    let ack = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(path);
    let ack = ack.trim().to_string();
    (!ack.is_empty()).then_some(ack)
}

fn setup_notification_key_bindings() -> Result<()> {
    let ack_signal_path = ack_signal_path()?;
    let ack_signal_path = super::utils::shell_escape(&ack_signal_path.to_string_lossy());

    let _ = Command::new("tmux")
        .args([
            "bind-key",
            NOTIFICATION_TRIGGER_KEY,
            "switch-client",
            "-T",
            NOTIFICATION_KEY_TABLE,
        ])
        .output();

    let _ = Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            NOTIFICATION_KEY_TABLE,
            "Escape",
            "switch-client",
            "-T",
            "root",
        ])
        .output();

    for index in 1..=MAX_NOTIFICATION_BINDINGS {
        let command = format!(
            "instance=#{{@aoe_notify_instance_{index}}}; \
target=#{{@aoe_notify_target_{index}}}; \
if [ -n \"$instance\" ] && [ -n \"$target\" ]; then \
printf '%s' \"$instance\" > {ack_path} && tmux switch-client -t \"$target\"; \
fi",
            ack_path = ack_signal_path,
            index = index,
        );
        let _ = Command::new("tmux")
            .args([
                "bind-key",
                "-T",
                NOTIFICATION_KEY_TABLE,
                &index.to_string(),
                "run-shell",
                &command,
                ";",
                "switch-client",
                "-T",
                "root",
            ])
            .output();
    }

    Ok(())
}

fn cleanup_notification_key_bindings() {
    let _ = Command::new("tmux")
        .args(["unbind-key", NOTIFICATION_TRIGGER_KEY])
        .output();
    let _ = Command::new("tmux")
        .args(["unbind-key", "-T", NOTIFICATION_KEY_TABLE, "Escape"])
        .output();
    for index in 1..=MAX_NOTIFICATION_BINDINGS {
        let _ = Command::new("tmux")
            .args([
                "unbind-key",
                "-T",
                NOTIFICATION_KEY_TABLE,
                &index.to_string(),
            ])
            .output();
    }
}

fn update_notification_options(
    profile: &str,
    session_names: &[String],
    states: &mut HashMap<String, MonitorSessionState>,
    acknowledged_instance_id: Option<&str>,
) -> Result<Duration> {
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
    let updates = build_notification_session_updates(session_names, &entries);
    let has_bindings = session_names
        .iter()
        .any(|session_name| !visible_notification_entries(&entries, session_name).is_empty());

    if has_bindings {
        setup_notification_key_bindings()?;
    } else {
        cleanup_notification_key_bindings();
    }
    apply_session_option_updates(&updates)?;

    Ok(poll_interval_for_statuses(
        instances.iter().map(|instance| instance.status),
    ))
}

fn clear_notification_options(session_names: &[String]) {
    cleanup_notification_key_bindings();

    for session_name in session_names {
        let _ = unset_session_option(session_name, NOTIFICATION_OPTION);
        let _ = unset_session_option(session_name, NOTIFICATION_HINT_OPTION);
        for index in 1..=MAX_NOTIFICATION_BINDINGS {
            let _ = unset_session_option(
                session_name,
                &notification_option_name("@aoe_notify_target", index),
            );
            let _ = unset_session_option(
                session_name,
                &notification_option_name("@aoe_notify_instance", index),
            );
        }
    }
}

pub fn ensure_notification_monitor(profile: &str) -> Result<()> {
    if get_server_option(NOTIFICATION_MONITOR_PID_OPTION)
        .as_deref()
        .is_some_and(pid_is_running)
    {
        return Ok(());
    }

    let current_exe = std::env::current_exe()?;
    let mut child = Command::new(current_exe);
    child
        .args(["tmux", "monitor-notifications", "--profile", profile])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let monitor = child.spawn()?;
    set_server_option(NOTIFICATION_MONITOR_PID_OPTION, &monitor.id().to_string())?;
    Ok(())
}

pub fn run_notification_monitor(profile: &str) -> Result<()> {
    let pid = std::process::id().to_string();
    let startup_deadline = Instant::now() + Duration::from_secs(2);
    let mut states = HashMap::<String, MonitorSessionState>::new();

    loop {
        match get_server_option(NOTIFICATION_MONITOR_PID_OPTION) {
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
        let poll_interval = match update_notification_options(
            profile,
            &session_names,
            &mut states,
            ack_signal.as_deref(),
        ) {
            Ok(interval) => interval,
            Err(err) => {
                tracing::debug!("Failed to refresh notification options: {}", err);
                clear_notification_options(&session_names);
                POLL_INTERVAL_WAITING
            }
        };

        if get_server_option(NOTIFICATION_MONITOR_PID_OPTION).as_deref() != Some(pid.as_str()) {
            break;
        }

        thread::sleep(poll_interval);
    }

    if get_server_option(NOTIFICATION_MONITOR_PID_OPTION).as_deref() == Some(pid.as_str()) {
        let sessions = list_aoe_sessions().unwrap_or_default();
        clear_notification_options(&sessions);
        let _ = unset_server_option(NOTIFICATION_MONITOR_PID_OPTION);
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
    let _ = unset_session_option(&session_name, NOTIFICATION_HINT_OPTION);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::config::SortOrder;
    use crate::session::Status;

    #[test]
    fn test_format_notification_text_with_multiple_sessions() {
        let entries = vec![
            NotificationEntry {
                instance_id: "alpha".to_string(),
                session_name: "aoe_alpha_1".to_string(),
                title: "alpha".to_string(),
                status: Status::Waiting,
            },
            NotificationEntry {
                instance_id: "beta".to_string(),
                session_name: "aoe_beta_2".to_string(),
                title: "beta".to_string(),
                status: Status::Idle,
            },
            NotificationEntry {
                instance_id: "gamma".to_string(),
                session_name: "aoe_gamma_3".to_string(),
                title: "gamma".to_string(),
                status: Status::Waiting,
            },
        ];

        assert_eq!(
            format_notification_text(&entries, "aoe_missing"),
            "[1] \u{25d0} alpha [2] \u{25cb} beta [3] \u{25d0} gamma"
        );
    }

    #[test]
    fn test_format_notification_text_excludes_self_and_renumbers() {
        let entries = vec![
            NotificationEntry {
                instance_id: "alpha".to_string(),
                session_name: "aoe_alpha_1".to_string(),
                title: "alpha".to_string(),
                status: Status::Waiting,
            },
            NotificationEntry {
                instance_id: "beta".to_string(),
                session_name: "aoe_beta_2".to_string(),
                title: "beta".to_string(),
                status: Status::Idle,
            },
        ];

        assert_eq!(
            format_notification_text(&entries, "aoe_alpha_1"),
            "[1] \u{25cb} beta"
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
            .into_iter()
            .map(|entry| entry.session_name)
            .collect();

        assert!(session_names.contains(&waiting_name));
        assert!(session_names.contains(&idle_visible_name));
        assert!(!session_names.contains(&idle_hidden_name));
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
    fn test_build_batched_session_option_args_supports_set_and_unset() {
        let args = build_batched_session_option_args(&[
            SessionOptionUpdate {
                session_name: "aoe_alpha".to_string(),
                option: "@aoe_waiting".to_string(),
                value: Some("[1] waiting".to_string()),
            },
            SessionOptionUpdate {
                session_name: "aoe_beta".to_string(),
                option: "@aoe_notification_hint".to_string(),
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
                "@aoe_notification_hint",
            ]
        );
    }
}
