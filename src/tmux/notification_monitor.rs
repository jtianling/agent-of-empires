//! Background notification monitor for tmux status bar.
//!
//! Polls session statuses and updates per-session `@aoe_waiting` tmux
//! user options so the status bar can display Waiting/Idle sessions.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::session::config::{load_config, SortOrder};
use crate::session::{flatten_tree, Group, GroupTree, Instance, Item, Status, Storage};

use super::status_bar::{
    get_server_option, list_aoe_sessions, pid_is_running, set_server_option, set_session_option,
    unset_server_option, unset_session_option,
};

const NOTIFICATION_MONITOR_PID_OPTION: &str = "@aoe_notification_monitor_pid";
const NOTIFICATION_OPTION: &str = "@aoe_waiting";
const NOTIFICATION_MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
struct NotificationEntry {
    session_name: String,
    title: String,
    index: usize,
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
        .enumerate()
        .map(|(index, (session_name, instance))| NotificationEntry {
            session_name,
            title: instance.title.clone(),
            index: index + 1,
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

fn should_notify_for_instance(instance: &Instance, collapsed_paths: &HashSet<&str>) -> bool {
    match instance.status {
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
        .filter(|instance| should_notify_for_instance(instance, &collapsed_paths))
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

fn format_notification_text(entries: &[NotificationEntry], current_session: &str) -> String {
    entries
        .iter()
        .filter(|entry| entry.session_name != current_session)
        .map(|entry| format!("[{}] {}", entry.index, entry.title))
        .collect::<Vec<_>>()
        .join(" ")
}

fn update_notification_options(profile: &str, session_names: &[String]) -> Result<()> {
    let storage = Storage::new(profile)?;
    let (instances, groups) = storage.load_with_groups()?;
    let existing_sessions: HashSet<String> = session_names.iter().cloned().collect();
    let sort_order = current_home_sort_order();
    let entries = build_notification_entries(&instances, &groups, sort_order, &existing_sessions);

    for session_name in session_names {
        let text = format_notification_text(&entries, session_name);
        set_session_option(session_name, NOTIFICATION_OPTION, &text)?;
    }

    Ok(())
}

fn clear_notification_options(session_names: &[String]) {
    for session_name in session_names {
        let _ = unset_session_option(session_name, NOTIFICATION_OPTION);
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

        let session_names = list_aoe_sessions()?;
        if session_names.is_empty() {
            break;
        }

        if let Err(err) = update_notification_options(profile, &session_names) {
            tracing::debug!("Failed to refresh notification options: {}", err);
            clear_notification_options(&session_names);
        }

        thread::sleep(NOTIFICATION_MONITOR_POLL_INTERVAL);
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
                session_name: "aoe_alpha_1".to_string(),
                title: "alpha".to_string(),
                index: 1,
            },
            NotificationEntry {
                session_name: "aoe_beta_2".to_string(),
                title: "beta".to_string(),
                index: 2,
            },
            NotificationEntry {
                session_name: "aoe_gamma_3".to_string(),
                title: "gamma".to_string(),
                index: 3,
            },
        ];

        assert_eq!(
            format_notification_text(&entries, "aoe_missing"),
            "[1] alpha [2] beta [3] gamma"
        );
    }

    #[test]
    fn test_format_notification_text_excludes_self() {
        let entries = vec![
            NotificationEntry {
                session_name: "aoe_alpha_1".to_string(),
                title: "alpha".to_string(),
                index: 1,
            },
            NotificationEntry {
                session_name: "aoe_beta_2".to_string(),
                title: "beta".to_string(),
                index: 2,
            },
        ];

        assert_eq!(
            format_notification_text(&entries, "aoe_alpha_1"),
            "[2] beta"
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
}
