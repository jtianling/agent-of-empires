//! tmux status bar configuration for aoe sessions

use anyhow::Result;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::session::Status;

const ACTIVE_PANE_TITLE_FORMAT: &str = "#T";
const CODEX_TITLE_MONITOR_PID_OPTION: &str = "@aoe_codex_title_monitor_pid";
const CODEX_TITLE_MONITOR_TITLE_OPTION: &str = "@aoe_codex_title_monitor_title";
const CODEX_TITLE_MONITOR_POLL_INTERVAL: Duration = Duration::from_millis(500);
const CODEX_WAITING_TITLE_PREFIX: &str = "\u{270b} ";

/// Information about a sandboxed session for status bar display.
pub struct SandboxDisplay {
    pub container_name: String,
}

pub fn managed_agent_pane_title(tool: &str, title: &str, status: Status) -> String {
    if tool == "codex" && status == Status::Waiting {
        format!("{}{}", CODEX_WAITING_TITLE_PREFIX, title)
    } else {
        title.to_string()
    }
}

/// Apply aoe-styled status bar configuration to a tmux session.
///
/// Sets tmux user options (@aoe_title, @aoe_branch, @aoe_sandbox) and configures
/// the status-right to display session information.
pub fn apply_status_bar(
    session_name: &str,
    title: &str,
    branch: Option<&str>,
    sandbox: Option<&SandboxDisplay>,
) -> Result<()> {
    // Set the session title as a tmux user option
    set_session_option(session_name, "@aoe_title", title)?;

    // Set branch if provided (for worktree sessions)
    if let Some(branch_name) = branch {
        set_session_option(session_name, "@aoe_branch", branch_name)?;
    }

    // Set sandbox info if running in docker container
    if let Some(sandbox_info) = sandbox {
        set_session_option(session_name, "@aoe_sandbox", &sandbox_info.container_name)?;
    }

    // Configure the status bar format using aoe's phosphor green theme
    // colour46 = bright green (matches aoe accent), colour48 = cyan (matches running)
    // colour235 = dark background
    //
    // Format: "aoe: Title | branch | [container] | 14:30"
    // - #{@aoe_title}: session title
    // - #{?#{@aoe_branch}, | #{@aoe_branch},}: conditional branch display
    // - #{?#{@aoe_sandbox}, [#{@aoe_sandbox}],}: conditional sandbox container display
    let status_format = concat!(
        " #[fg=colour46,bold]aoe#[fg=colour252,nobold]: ",
        "#{@aoe_title}",
        "#{?#{@aoe_branch}, #[fg=colour48]| #{@aoe_branch}#[fg=colour252],}",
        "#{?#{@aoe_sandbox}, #[fg=colour214]⬡ #{@aoe_sandbox}#[fg=colour252],}",
        " | %H:%M "
    );

    set_session_option(session_name, "status-right", status_format)?;
    set_session_option(session_name, "status-right-length", "80")?;

    // Dark background with light text - matches aoe phosphor theme
    set_session_option(session_name, "status-style", "bg=colour235,fg=colour252")?;
    set_session_option(
        session_name,
        "status-left",
        " #[fg=colour46,bold]#S#[fg=colour252,nobold] │ #[fg=colour245]Ctrl+q#[fg=colour240]/#[fg=colour245]Ctrl+b d#[fg=colour240] detach #[fg=colour245]Ctrl+b n/p#[fg=colour240] switch #[fg=colour245]Ctrl+b 1-9#[fg=colour240] jump ",
    )?;
    set_session_option(session_name, "status-left-length", "80")?;

    Ok(())
}

/// Set a tmux option for a specific session.
fn set_session_option(session_name: &str, option: &str, value: &str) -> Result<()> {
    let output = Command::new("tmux")
        .args(["set-option", "-t", session_name, option, value])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Don't fail on option errors - status bar is non-critical
        tracing::debug!("Failed to set tmux option {}: {}", option, stderr);
    }

    Ok(())
}

/// Set a tmux window option for the active window in a specific session.
fn set_window_option(session_name: &str, option: &str, value: &str) -> Result<()> {
    let output = Command::new("tmux")
        .args(["set-window-option", "-t", session_name, option, value])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::debug!("Failed to set tmux window option {}: {}", option, stderr);
    }

    Ok(())
}

/// Apply mouse support option to a tmux session.
/// When enabled, scrolling with the mouse wheel enters copy mode.
pub fn apply_mouse_option(session_name: &str, enabled: bool) -> Result<()> {
    let value = if enabled { "on" } else { "off" };
    set_session_option(session_name, "mouse", value)
}

/// Configure tmux so the outer terminal title follows the active pane title
/// while the user is attached to this AoE-managed session.
fn enable_title_passthrough(session_name: &str) -> Result<()> {
    set_session_option(session_name, "set-titles", "on")?;
    set_session_option(session_name, "set-titles-string", ACTIVE_PANE_TITLE_FORMAT)?;
    set_window_option(session_name, "allow-set-title", "on")?;
    Ok(())
}

/// Set the pane title for a tmux session via `select-pane -T`.
///
/// This gives agents that don't set their own OSC 0 title (e.g. Codex CLI)
/// a useful default instead of showing the hostname.
fn set_initial_pane_title(session_name: &str, title: &str) -> Result<()> {
    set_pane_title(session_name, title)
}

fn set_pane_title(target: &str, title: &str) -> Result<()> {
    let output = Command::new("tmux")
        .args(["select-pane", "-t", target, "-T", title])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::debug!("Failed to set pane title for {}: {}", target, stderr);
    }

    Ok(())
}

fn unset_session_option(session_name: &str, option: &str) -> Result<()> {
    let output = Command::new("tmux")
        .args(["set-option", "-t", session_name, "-u", option])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::debug!("Failed to unset tmux option {}: {}", option, stderr);
    }

    Ok(())
}

fn session_exists(session_name: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", session_name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn capture_pane(target: &str, lines: usize) -> Result<String> {
    let output = Command::new("tmux")
        .args([
            "capture-pane",
            "-t",
            target,
            "-p",
            "-S",
            &format!("-{}", lines),
        ])
        .output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Ok(String::new())
    }
}

fn pid_is_running(pid: &str) -> bool {
    if pid.trim().is_empty() {
        return false;
    }

    Command::new("/bin/kill")
        .args(["-0", pid])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn ensure_codex_title_monitor(session_name: &str, title: &str) -> Result<()> {
    set_session_option(session_name, CODEX_TITLE_MONITOR_TITLE_OPTION, title)?;

    if get_session_option(session_name, CODEX_TITLE_MONITOR_PID_OPTION)
        .as_deref()
        .is_some_and(pid_is_running)
    {
        return Ok(());
    }

    let current_exe = std::env::current_exe()?;
    let mut child = Command::new(current_exe);
    child
        .args(["tmux", "monitor-codex-title", "--session", session_name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let monitor = child.spawn()?;
    set_session_option(
        session_name,
        CODEX_TITLE_MONITOR_PID_OPTION,
        &monitor.id().to_string(),
    )?;
    Ok(())
}

pub fn run_codex_title_monitor(session_name: &str) -> Result<()> {
    let pid = std::process::id().to_string();
    let startup_deadline = Instant::now() + Duration::from_secs(2);
    let mut last_title: Option<String> = None;

    while session_exists(session_name) {
        match get_session_option(session_name, CODEX_TITLE_MONITOR_PID_OPTION) {
            Some(owner) if owner == pid => {}
            Some(_) => break,
            None if Instant::now() <= startup_deadline => {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            None => break,
        }

        let Some(title) = get_session_option(session_name, CODEX_TITLE_MONITOR_TITLE_OPTION) else {
            break;
        };
        let pane_content = capture_pane(session_name, 50)?;
        let status = crate::tmux::status_detection::detect_codex_status(&pane_content);
        let desired = managed_agent_pane_title("codex", &title, status);
        if last_title.as_deref() != Some(desired.as_str()) {
            set_pane_title(session_name, &desired)?;
            last_title = Some(desired);
        }
        thread::sleep(CODEX_TITLE_MONITOR_POLL_INTERVAL);
    }

    if get_session_option(session_name, CODEX_TITLE_MONITOR_PID_OPTION).as_deref() == Some(&pid) {
        let _ = unset_session_option(session_name, CODEX_TITLE_MONITOR_PID_OPTION);
    }

    Ok(())
}

/// Apply all configured tmux options to a session.
/// This is a unified entry point that applies status bar styling and mouse settings.
pub fn apply_all_tmux_options(
    session_name: &str,
    title: &str,
    branch: Option<&str>,
    sandbox: Option<&SandboxDisplay>,
) {
    use crate::session::config::{should_apply_tmux_mouse, should_apply_tmux_status_bar};

    if should_apply_tmux_status_bar() {
        if let Err(e) = apply_status_bar(session_name, title, branch, sandbox) {
            tracing::debug!("Failed to apply tmux status bar: {}", e);
        }
    }

    if let Some(mouse_enabled) = should_apply_tmux_mouse() {
        if let Err(e) = apply_mouse_option(session_name, mouse_enabled) {
            tracing::debug!("Failed to apply tmux mouse option: {}", e);
        }
    }

    if let Err(e) = enable_title_passthrough(session_name) {
        tracing::debug!("Failed to enable tmux title passthrough: {}", e);
    }

    // Set an initial pane title so agents that don't write their own OSC 0
    // (e.g. Codex CLI) show something meaningful instead of the hostname.
    // Agents that do set titles (Claude Code, Gemini) will overwrite this.
    if let Err(e) = set_initial_pane_title(session_name, title) {
        tracing::debug!("Failed to set initial pane title: {}", e);
    }
}

/// Session info retrieved from tmux user options.
pub struct SessionInfo {
    pub title: String,
    pub branch: Option<String>,
    pub sandbox: Option<String>,
}

/// Get session info for the current tmux session (used by `aoe tmux-status` command).
/// Returns structured session info for use in user's custom tmux status bar.
pub fn get_session_info_for_current() -> Option<SessionInfo> {
    let session_name = crate::tmux::get_current_session_name()?;

    // Check if this is an aoe session
    if !session_name.starts_with(crate::tmux::SESSION_PREFIX) {
        return None;
    }

    // Try to get the aoe title from tmux user option
    let title = get_session_option(&session_name, "@aoe_title").unwrap_or_else(|| {
        // Fallback: extract title from session name
        // Session names are: aoe_<title>_<id>
        let name_without_prefix = session_name
            .strip_prefix(crate::tmux::SESSION_PREFIX)
            .unwrap_or(&session_name);
        if let Some(last_underscore) = name_without_prefix.rfind('_') {
            name_without_prefix[..last_underscore].to_string()
        } else {
            name_without_prefix.to_string()
        }
    });

    let branch = get_session_option(&session_name, "@aoe_branch");
    let sandbox = get_session_option(&session_name, "@aoe_sandbox");

    Some(SessionInfo {
        title,
        branch,
        sandbox,
    })
}

/// Get formatted status string for the current tmux session.
/// Returns a plain text string like "aoe: Title | branch | [container]"
pub fn get_status_for_current_session() -> Option<String> {
    let info = get_session_info_for_current()?;

    let mut result = format!("aoe: {}", info.title);

    if let Some(b) = &info.branch {
        result.push_str(" | ");
        result.push_str(b);
    }

    if let Some(s) = &info.sandbox {
        result.push_str(" [");
        result.push_str(s);
        result.push(']');
    }

    Some(result)
}

/// Get a tmux option value for a session.
fn get_session_option(session_name: &str, option: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["show-options", "-t", session_name, "-v", option])
        .output()
        .ok()?;

    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Status;

    #[test]
    fn test_active_pane_title_format_uses_tmux_pane_title() {
        assert_eq!(ACTIVE_PANE_TITLE_FORMAT, "#T");
    }

    #[test]
    fn test_managed_agent_pane_title_adds_hand_only_for_codex_waiting() {
        assert_eq!(
            managed_agent_pane_title("codex", "My Session", Status::Waiting),
            "\u{270b} My Session"
        );
        assert_eq!(
            managed_agent_pane_title("codex", "My Session", Status::Running),
            "My Session"
        );
        assert_eq!(
            managed_agent_pane_title("opencode", "My Session", Status::Waiting),
            "My Session"
        );
    }

    #[test]
    fn test_get_status_returns_none_for_non_tmux() {
        // When not in tmux, get_current_session_name returns None
        // so get_status_for_current_session should also return None
        // This test just verifies the function doesn't panic
        let _ = get_status_for_current_session();
    }
}
