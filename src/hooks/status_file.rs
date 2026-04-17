//! Status file I/O for hooks-based agent status detection.
//!
//! Agent hooks write `running`, `waiting`, or `idle` to a well-known
//! file path so AoE can detect agent status without parsing tmux pane content.
//!
//! ## Freshness contract
//!
//! The hook writer (a shell snippet installed in the agent's settings.json)
//! updates `status` on every hook event. In steady-state the file is rewritten
//! every few seconds (any `PreToolUse` / `UserPromptSubmit` / `Notification`
//! event refreshes mtime). A `Stop` event is supposed to close each turn by
//! writing `idle`, but several real-world paths skip `Stop` (user presses Esc,
//! client-side slash command, agent crash). In those cases the file is left at
//! `running` or `waiting` indefinitely.
//!
//! To protect against this, callers that drive live status display MUST use
//! [`read_hook_status_with_mtime`] and gate the result through
//! [`is_hook_fresh`] (or the combined helper in the hooks module). A status is
//! considered authoritative only when the file's mtime is within
//! [`HOOK_STATUS_FRESHNESS_WINDOW`] of the current time. Stale files are
//! treated as if the hook file did not exist, which lets the caller fall
//! through to content-based detection.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::session::Status;

use super::{HOOK_STATUS_BASE, HOOK_STATUS_FRESHNESS_WINDOW};

/// Return the directory for a given instance's hook status file.
pub fn hook_status_dir(instance_id: &str) -> PathBuf {
    PathBuf::from(HOOK_STATUS_BASE).join(instance_id)
}

fn parse_hook_status(content: &str) -> Option<Status> {
    match content.trim() {
        "running" => Some(Status::Running),
        "waiting" => Some(Status::Waiting),
        "idle" => Some(Status::Idle),
        other => {
            tracing::warn!("Unexpected hook status value: {:?}", other);
            None
        }
    }
}

/// Read the hook-written status file for the given instance along with its
/// last-modified time.
///
/// Returns `None` if the file doesn't exist, cannot be read, contains an
/// unrecognized value, or has no readable mtime. Callers that need to gate
/// on freshness should combine this with [`is_hook_fresh`].
pub fn read_hook_status_with_mtime(instance_id: &str) -> Option<(Status, SystemTime)> {
    let status_path = hook_status_dir(instance_id).join("status");
    let content = std::fs::read_to_string(&status_path).ok()?;
    let status = parse_hook_status(&content)?;
    let mtime = std::fs::metadata(&status_path).ok()?.modified().ok()?;
    Some((status, mtime))
}

/// Read the hook-written status file for the given instance.
///
/// Returns `None` if the file doesn't exist. When `Some`, the hook is
/// actively tracking the session and shell detection is unreliable
/// (wrapper scripts may keep a shell alive). Callers should still use
/// `is_pane_dead()` to detect truly dead panes.
///
/// Note: this variant does NOT perform a freshness check. It is intended for
/// callers that only care about presence/absence (e.g. deciding whether
/// `is_pane_running_shell` heuristics should be bypassed). Callers that use
/// the returned status to drive a live UI state MUST go through
/// [`read_hook_status_with_mtime`] plus [`is_hook_fresh`].
pub fn read_hook_status(instance_id: &str) -> Option<Status> {
    read_hook_status_with_mtime(instance_id).map(|(status, _)| status)
}

/// Return `true` if a hook status file with the given `mtime` should be
/// treated as authoritative.
///
/// A file is fresh when its mtime is within [`HOOK_STATUS_FRESHNESS_WINDOW`]
/// of `SystemTime::now()`. "Future" mtimes (e.g. clock skew between host and
/// container filesystem) are treated as fresh so callers don't spuriously
/// reject a just-written file.
pub fn is_hook_fresh(mtime: SystemTime) -> bool {
    is_hook_fresh_at(mtime, SystemTime::now(), HOOK_STATUS_FRESHNESS_WINDOW)
}

fn is_hook_fresh_at(mtime: SystemTime, now: SystemTime, window: Duration) -> bool {
    match now.duration_since(mtime) {
        Ok(age) => age <= window,
        Err(_) => true,
    }
}

/// Read the agent session ID written by hooks (e.g. `CLAUDE_SESSION_ID`).
/// Returns `None` if the file doesn't exist or is empty.
pub fn read_hook_session_id(instance_id: &str) -> Option<String> {
    let path = hook_status_dir(instance_id).join("session_id");
    let content = std::fs::read_to_string(&path).ok()?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Remove the hook status directory for a given instance (cleanup on stop/delete).
pub fn cleanup_hook_status_dir(instance_id: &str) {
    let dir = hook_status_dir(instance_id);
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!("Failed to cleanup hook status dir {}: {}", dir.display(), e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn setup_status_file(instance_id: &str, content: &str) -> PathBuf {
        let dir = hook_status_dir(instance_id);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("status");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        dir
    }

    #[test]
    fn test_read_running_status() {
        let id = "test_read_running";
        let dir = setup_status_file(id, "running");
        assert_eq!(read_hook_status(id), Some(Status::Running));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_read_waiting_status() {
        let id = "test_read_waiting";
        let dir = setup_status_file(id, "waiting");
        assert_eq!(read_hook_status(id), Some(Status::Waiting));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_read_idle_status() {
        let id = "test_read_idle";
        let dir = setup_status_file(id, "idle");
        assert_eq!(read_hook_status(id), Some(Status::Idle));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_read_waiting_with_newline() {
        let id = "test_read_newline";
        let dir = setup_status_file(id, "waiting\n");
        assert_eq!(read_hook_status(id), Some(Status::Waiting));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_read_missing_file() {
        assert_eq!(read_hook_status("nonexistent_instance_id"), None);
    }

    #[test]
    fn test_read_dangling_symlink() {
        let id = "test_dangling_symlink";
        let dir = hook_status_dir(id);
        fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink("/nonexistent/target", dir.join("status")).unwrap();
        assert_eq!(read_hook_status(id), None);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_read_unexpected_content() {
        let id = "test_read_unexpected";
        let dir = setup_status_file(id, "something_else");
        assert_eq!(read_hook_status(id), None);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_cleanup_existing_dir() {
        let id = "test_cleanup_existing";
        let dir = setup_status_file(id, "running");
        assert!(dir.exists());
        cleanup_hook_status_dir(id);
        assert!(!dir.exists());
    }

    #[test]
    fn test_cleanup_nonexistent_dir() {
        // Should not panic
        cleanup_hook_status_dir("nonexistent_cleanup_test");
    }

    #[test]
    fn test_hook_status_dir_path() {
        let dir = hook_status_dir("abc123");
        assert_eq!(dir, PathBuf::from("/tmp/aoe-hooks/abc123"));
    }

    #[test]
    fn test_read_hook_status_with_mtime_returns_mtime() {
        let id = "test_mtime_match";
        let dir = setup_status_file(id, "running");
        let path = dir.join("status");

        let (status, mtime) = read_hook_status_with_mtime(id).expect("status should be readable");
        assert_eq!(status, Status::Running);
        let meta_mtime = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(mtime, meta_mtime);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_read_hook_status_with_mtime_missing_file() {
        assert!(read_hook_status_with_mtime("nonexistent_mtime_test").is_none());
    }

    #[test]
    fn test_is_hook_fresh_recent_mtime() {
        let now = SystemTime::now();
        let mtime = now - Duration::from_secs(1);
        assert!(is_hook_fresh_at(mtime, now, Duration::from_secs(30)));
    }

    #[test]
    fn test_is_hook_fresh_boundary_equal_to_window() {
        let now = SystemTime::now();
        let mtime = now - Duration::from_secs(30);
        assert!(is_hook_fresh_at(mtime, now, Duration::from_secs(30)));
    }

    #[test]
    fn test_is_hook_fresh_stale_mtime() {
        let now = SystemTime::now();
        let mtime = now - Duration::from_secs(3600);
        assert!(!is_hook_fresh_at(mtime, now, Duration::from_secs(30)));
    }

    #[test]
    fn test_is_hook_fresh_future_mtime_treated_as_fresh() {
        let now = SystemTime::now();
        let mtime = now + Duration::from_secs(60);
        assert!(is_hook_fresh_at(mtime, now, Duration::from_secs(30)));
    }

    #[test]
    fn test_is_hook_fresh_live_clock() {
        let mtime = SystemTime::now();
        assert!(is_hook_fresh(mtime));
    }
}
