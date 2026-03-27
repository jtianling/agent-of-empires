//! tmux integration module

pub(crate) mod notification_monitor;
mod session;
pub mod status_bar;
pub(crate) mod status_detection;
pub(crate) mod utils;

pub use notification_monitor::write_ack_signal;
pub use session::{split_window_right, Session};
pub use status_bar::{get_session_info_for_current, get_status_for_current_session};
pub use status_detection::detect_status_from_content;

use std::collections::HashMap;
use std::process::Command;
use std::sync::RwLock;
use std::time::{Duration, Instant};

pub const SESSION_PREFIX: &str = "aoe_";

/// Pre-fetched pane metadata from a single `tmux list-panes -a` call.
#[derive(Debug, Clone)]
pub struct PaneMetadata {
    pub pane_dead: bool,
    pub pane_current_command: Option<String>,
}

static SESSION_CACHE: RwLock<SessionCache> = RwLock::new(SessionCache {
    data: None,
    time: None,
});
static PANE_INFO_CACHE: RwLock<PaneInfoCache> = RwLock::new(PaneInfoCache {
    data: None,
    time: None,
});

struct SessionCache {
    data: Option<HashMap<String, SessionActivity>>,
    time: Option<Instant>,
}

struct PaneInfoCache {
    data: Option<HashMap<String, Vec<PaneInfo>>>,
    time: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionActivity {
    pub session_activity: i64,
    pub window_activity: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub pane_index: u32,
    pub pane_id: String,
    pub pane_title: String,
    pub current_command: String,
    pub is_dead: bool,
    pub pane_pid: Option<u32>,
}

pub fn refresh_session_cache() {
    let output = Command::new("tmux")
        .args([
            "list-sessions",
            "-F",
            "#{session_name}\t#{session_activity}\t#{window_activity}",
        ])
        .output();

    let new_data = match output {
        Ok(out) if out.status.success() => Some(parse_session_cache_output(&out.stdout)),
        _ => None,
    };

    if let Ok(mut cache) = SESSION_CACHE.write() {
        cache.data = new_data;
        cache.time = Some(Instant::now());
    }
}

pub fn refresh_pane_info_cache() {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}\t#{pane_index}\t#{pane_id}\t#{pane_title}\t#{pane_current_command}\t#{pane_dead}\t#{pane_pid}",
        ])
        .output();

    let new_data = match output {
        Ok(out) if out.status.success() => Some(parse_pane_info_cache_output(&out.stdout)),
        _ => None,
    };

    if let Ok(mut cache) = PANE_INFO_CACHE.write() {
        cache.data = new_data;
        cache.time = Some(Instant::now());
    }
}

/// Batch-fetch pane metadata for all aoe sessions in a single tmux subprocess call.
/// Returns a map from session name to metadata for the first window's first pane.
pub fn batch_pane_metadata() -> HashMap<String, PaneMetadata> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}\t#{pane_index}\t#{pane_dead}\t#{pane_current_command}",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            parse_pane_metadata(&stdout)
        }
        _ => HashMap::new(),
    }
}

/// Parse the output of `tmux list-panes -a` into a map of session name to pane metadata.
/// Filters to aoe sessions, pane index 0, and takes only the first window per session.
fn parse_pane_metadata(output: &str) -> HashMap<String, PaneMetadata> {
    let mut map = HashMap::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 4 {
            continue;
        }

        let session_name = parts[0];
        if !session_name.starts_with(SESSION_PREFIX) {
            continue;
        }

        // Only take pane 0 (the agent pane). aoe pins pane-base-index to 0.
        if parts[1] != "0" {
            continue;
        }

        // First occurrence per session = first window's pane 0 (list-panes
        // returns windows in index order).
        if map.contains_key(session_name) {
            continue;
        }

        map.insert(
            session_name.to_string(),
            PaneMetadata {
                pane_dead: parts[2] == "1",
                pane_current_command: if parts[3].is_empty() {
                    None
                } else {
                    Some(parts[3].to_string())
                },
            },
        );
    }

    map
}

pub fn session_exists_from_cache(name: &str) -> Option<bool> {
    let cache = SESSION_CACHE.read().ok()?;

    // Cache valid for 2 seconds
    if cache
        .time
        .map(|t| t.elapsed() > Duration::from_secs(2))
        .unwrap_or(true)
    {
        return None;
    }

    cache.data.as_ref().map(|m| m.contains_key(name))
}

pub fn get_cached_window_activity(name: &str) -> Option<i64> {
    let cache = SESSION_CACHE.read().ok()?;
    if is_cache_stale(cache.time) {
        return None;
    }

    cache
        .data
        .as_ref()
        .and_then(|m| m.get(name))
        .map(|activity| activity.window_activity)
}

pub fn get_cached_pane_info(name: &str) -> Option<PaneInfo> {
    let cache = PANE_INFO_CACHE.read().ok()?;
    if is_cache_stale(cache.time) {
        return None;
    }

    cache
        .data
        .as_ref()
        .and_then(|m| m.get(name))
        .and_then(|panes| panes.first())
        .cloned()
}

pub fn get_all_cached_pane_infos(session_name: &str) -> Option<Vec<PaneInfo>> {
    let cache = PANE_INFO_CACHE.read().ok()?;
    if is_cache_stale(cache.time) {
        return None;
    }

    cache
        .data
        .as_ref()
        .and_then(|m| m.get(session_name))
        .cloned()
}

pub fn get_current_session_name() -> Option<String> {
    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{session_name}"])
        .output()
        .ok()?;

    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

pub fn get_current_client_name() -> Option<String> {
    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{client_name}"])
        .output()
        .ok()?;

    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

pub fn get_tty_name() -> Option<String> {
    let output = Command::new("tty")
        .stdin(std::process::Stdio::inherit())
        .output()
        .ok()?;
    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() && name != "not a tty" {
            return Some(name);
        }
    }
    None
}

pub fn is_tmux_available() -> bool {
    Command::new("tmux").arg("-V").output().is_ok()
}

fn is_agent_available(agent: &crate::agents::AgentDef) -> bool {
    use crate::agents::DetectionMethod;
    match &agent.detection {
        DetectionMethod::Which(binary) => Command::new("which")
            .arg(binary)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false),
        DetectionMethod::RunWithArg(binary, arg) => Command::new(binary).arg(arg).output().is_ok(),
    }
}

fn is_cache_stale(time: Option<Instant>) -> bool {
    time.map(|t| t.elapsed() > Duration::from_secs(2))
        .unwrap_or(true)
}

fn parse_session_cache_output(output: &[u8]) -> HashMap<String, SessionActivity> {
    let stdout = String::from_utf8_lossy(output);
    let mut map = HashMap::new();

    for line in stdout.lines() {
        let mut parts = line.splitn(3, '\t');
        let Some(name) = parts.next() else {
            continue;
        };
        let Some(session_activity) = parts.next() else {
            continue;
        };
        let Some(window_activity) = parts.next() else {
            continue;
        };

        map.insert(
            name.to_string(),
            SessionActivity {
                session_activity: session_activity.parse().unwrap_or(0),
                window_activity: window_activity.parse().unwrap_or(0),
            },
        );
    }

    map
}

fn parse_pane_info_cache_output(output: &[u8]) -> HashMap<String, Vec<PaneInfo>> {
    let stdout = String::from_utf8_lossy(output);
    let mut panes_by_session: HashMap<String, Vec<PaneInfo>> = HashMap::new();

    for line in stdout.lines() {
        let mut parts = line.splitn(7, '\t');
        let Some(session_name) = parts.next() else {
            continue;
        };
        if !session_name.starts_with(SESSION_PREFIX) {
            continue;
        }

        let Some(pane_index_str) = parts.next() else {
            continue;
        };
        let Some(pane_id) = parts.next() else {
            continue;
        };
        let Some(pane_title) = parts.next() else {
            continue;
        };
        let Some(current_command) = parts.next() else {
            continue;
        };
        let Some(is_dead) = parts.next() else {
            continue;
        };
        let Some(pane_pid) = parts.next() else {
            continue;
        };

        let pane_index = pane_index_str.parse().unwrap_or(u32::MAX);
        let info = PaneInfo {
            pane_index,
            pane_id: pane_id.to_string(),
            pane_title: pane_title.to_string(),
            current_command: current_command.to_string(),
            is_dead: is_dead.trim() == "1",
            pane_pid: pane_pid.trim().parse().ok(),
        };

        panes_by_session
            .entry(session_name.to_string())
            .or_default()
            .push(info);
    }

    for panes in panes_by_session.values_mut() {
        panes.sort_by_key(|p| p.pane_index);
    }

    panes_by_session
}

#[derive(Debug, Clone)]
pub struct AvailableTools {
    available: Vec<&'static str>,
}

impl AvailableTools {
    pub fn detect() -> Self {
        let available = crate::agents::AGENTS
            .iter()
            .filter(|a| is_agent_available(a))
            .map(|a| a.name)
            .collect();
        Self { available }
    }

    pub fn any_available(&self) -> bool {
        !self.available.is_empty()
    }

    pub fn available_list(&self) -> Vec<&'static str> {
        self.available.clone()
    }

    #[cfg(test)]
    pub fn with_tools(tools: &[&'static str]) -> Self {
        Self {
            available: tools.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_session_cache_output_includes_window_activity() {
        let parsed =
            parse_session_cache_output(b"aoe_one\t11\t21\naoe_two\t12\t22\nother\t13\t23\n");

        assert_eq!(
            parsed.get("aoe_one"),
            Some(&SessionActivity {
                session_activity: 11,
                window_activity: 21
            })
        );
        assert_eq!(
            parsed.get("aoe_two"),
            Some(&SessionActivity {
                session_activity: 12,
                window_activity: 22
            })
        );
        assert_eq!(
            parsed.get("other"),
            Some(&SessionActivity {
                session_activity: 13,
                window_activity: 23
            })
        );
    }

    #[test]
    fn test_parse_pane_info_cache_output_filters_non_aoe_sessions() {
        let parsed = parse_pane_info_cache_output(
            b"aoe_alpha\t0\t%1\talpha title\tcodex\t0\t101\nother\t0\t%2\tother title\tbash\t1\t202\n",
        );

        assert_eq!(parsed.len(), 1);
        let panes = parsed.get("aoe_alpha").unwrap();
        assert_eq!(panes.len(), 1);
        assert_eq!(
            panes[0],
            PaneInfo {
                pane_index: 0,
                pane_id: "%1".to_string(),
                pane_title: "alpha title".to_string(),
                current_command: "codex".to_string(),
                is_dead: false,
                pane_pid: Some(101),
            }
        );
    }

    #[test]
    fn test_parse_pane_info_cache_output_stores_all_panes_sorted() {
        let parsed = parse_pane_info_cache_output(
            b"aoe_alpha\t2\t%3\tright pane\tbash\t0\t300\naoe_alpha\t0\t%1\tagent pane\tcodex\t0\t200\n",
        );

        let panes = parsed.get("aoe_alpha").unwrap();
        assert_eq!(panes.len(), 2);
        assert_eq!(
            panes[0],
            PaneInfo {
                pane_index: 0,
                pane_id: "%1".to_string(),
                pane_title: "agent pane".to_string(),
                current_command: "codex".to_string(),
                is_dead: false,
                pane_pid: Some(200),
            }
        );
        assert_eq!(
            panes[1],
            PaneInfo {
                pane_index: 2,
                pane_id: "%3".to_string(),
                pane_title: "right pane".to_string(),
                current_command: "bash".to_string(),
                is_dead: false,
                pane_pid: Some(300),
            }
        );
    }

    #[test]
    fn test_parse_pane_metadata_basic() {
        let output = "aoe_my_proj_abc12345\t0\t0\tclaude\n";
        let map = parse_pane_metadata(output);
        assert_eq!(map.len(), 1);
        let meta = map.get("aoe_my_proj_abc12345").unwrap();
        assert!(!meta.pane_dead);
        assert_eq!(meta.pane_current_command.as_deref(), Some("claude"));
    }

    #[test]
    fn test_parse_pane_metadata_dead_pane() {
        let output = "aoe_proj_abc12345\t0\t1\tbash\n";
        let map = parse_pane_metadata(output);
        let meta = map.get("aoe_proj_abc12345").unwrap();
        assert!(meta.pane_dead);
    }

    #[test]
    fn test_parse_pane_metadata_filters_non_aoe_sessions() {
        let output = "\
user_session\t0\t0\tbash\n\
aoe_proj_abc12345\t0\t0\tclaude\n\
my_tmux\t0\t0\tvim\n";
        let map = parse_pane_metadata(output);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("aoe_proj_abc12345"));
    }

    #[test]
    fn test_parse_pane_metadata_filters_non_zero_panes() {
        let output = "\
aoe_proj_abc12345\t0\t0\tclaude\n\
aoe_proj_abc12345\t1\t0\tbash\n";
        let map = parse_pane_metadata(output);
        assert_eq!(map.len(), 1);
        let meta = map.get("aoe_proj_abc12345").unwrap();
        assert_eq!(meta.pane_current_command.as_deref(), Some("claude"));
    }

    #[test]
    fn test_parse_pane_metadata_first_window_wins() {
        // Two windows both have pane 0, first window's data should be kept
        let output = "\
aoe_proj_abc12345\t0\t0\tclaude\n\
aoe_proj_abc12345\t0\t1\tbash\n";
        let map = parse_pane_metadata(output);
        assert_eq!(map.len(), 1);
        let meta = map.get("aoe_proj_abc12345").unwrap();
        assert!(!meta.pane_dead);
        assert_eq!(meta.pane_current_command.as_deref(), Some("claude"));
    }

    #[test]
    fn test_parse_pane_metadata_empty_output() {
        assert!(parse_pane_metadata("").is_empty());
    }

    #[test]
    fn test_parse_pane_metadata_malformed_lines() {
        let output = "\
too\tfew\tfields\n\
aoe_proj_abc12345\t0\t0\tclaude\n\
\n";
        let map = parse_pane_metadata(output);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_parse_pane_metadata_empty_command() {
        let output = "aoe_proj_abc12345\t0\t0\t\n";
        let map = parse_pane_metadata(output);
        let meta = map.get("aoe_proj_abc12345").unwrap();
        assert!(meta.pane_current_command.is_none());
    }

    #[test]
    fn test_parse_pane_metadata_multiple_sessions() {
        let output = "\
aoe_proj_a_abc12345\t0\t0\tclaude\n\
aoe_proj_b_def67890\t0\t0\topencode\n\
aoe_proj_c_ghi11111\t0\t1\tbash\n";
        let map = parse_pane_metadata(output);
        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get("aoe_proj_a_abc12345")
                .unwrap()
                .pane_current_command
                .as_deref(),
            Some("claude")
        );
        assert_eq!(
            map.get("aoe_proj_b_def67890")
                .unwrap()
                .pane_current_command
                .as_deref(),
            Some("opencode")
        );
        assert!(map.get("aoe_proj_c_ghi11111").unwrap().pane_dead);
    }
}
