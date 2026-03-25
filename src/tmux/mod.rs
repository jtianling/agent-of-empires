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
    data: Option<HashMap<String, PaneInfo>>,
    time: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionActivity {
    pub session_activity: i64,
    pub window_activity: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
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
            "#{session_name}\t#{pane_index}\t#{pane_title}\t#{pane_current_command}\t#{pane_dead}\t#{pane_pid}",
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

    cache.data.as_ref().and_then(|m| m.get(name)).cloned()
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

fn parse_pane_info_cache_output(output: &[u8]) -> HashMap<String, PaneInfo> {
    let stdout = String::from_utf8_lossy(output);
    let mut panes_by_session: HashMap<String, (u32, PaneInfo)> = HashMap::new();

    for line in stdout.lines() {
        let mut parts = line.splitn(6, '\t');
        let Some(session_name) = parts.next() else {
            continue;
        };
        if !session_name.starts_with(SESSION_PREFIX) {
            continue;
        }

        let Some(pane_index) = parts.next() else {
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

        let pane_index = pane_index.parse().unwrap_or(u32::MAX);
        let info = PaneInfo {
            pane_title: pane_title.to_string(),
            current_command: current_command.to_string(),
            is_dead: is_dead.trim() == "1",
            pane_pid: pane_pid.trim().parse().ok(),
        };

        match panes_by_session.get(session_name) {
            Some((existing_index, _)) if *existing_index <= pane_index => {}
            _ => {
                panes_by_session.insert(session_name.to_string(), (pane_index, info));
            }
        }
    }

    panes_by_session
        .into_iter()
        .map(|(session_name, (_, info))| (session_name, info))
        .collect()
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
            b"aoe_alpha\t0\talpha title\tcodex\t0\t101\nother\t0\tother title\tbash\t1\t202\n",
        );

        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed.get("aoe_alpha"),
            Some(&PaneInfo {
                pane_title: "alpha title".to_string(),
                current_command: "codex".to_string(),
                is_dead: false,
                pane_pid: Some(101),
            })
        );
    }

    #[test]
    fn test_parse_pane_info_cache_output_prefers_lowest_pane_index() {
        let parsed = parse_pane_info_cache_output(
            b"aoe_alpha\t2\tright pane\tbash\t0\t300\naoe_alpha\t0\tagent pane\tcodex\t0\t200\n",
        );

        assert_eq!(
            parsed.get("aoe_alpha"),
            Some(&PaneInfo {
                pane_title: "agent pane".to_string(),
                current_command: "codex".to_string(),
                is_dead: false,
                pane_pid: Some(200),
            })
        );
    }
}
