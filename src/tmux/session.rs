//! tmux session management

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::process::Command;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use super::{
    get_cached_pane_info, refresh_session_cache, session_exists_from_cache,
    utils::{
        append_remain_on_exit_args, append_store_pane_id_args, get_agent_pane_id, is_pane_dead,
        is_pane_running_shell,
    },
    SESSION_PREFIX,
};
use crate::cli::truncate_id;
use crate::process;
use crate::session::Status;

static CAPTURE_CACHE: RwLock<CaptureCache> = RwLock::new(CaptureCache { data: None });

const CAPTURE_CACHE_TTL: Duration = Duration::from_millis(500);

struct CaptureCache {
    data: Option<HashMap<String, CaptureCacheEntry>>,
}

#[derive(Clone)]
struct CaptureCacheEntry {
    content: String,
    timestamp: Instant,
    line_count: usize,
}

pub struct Session {
    name: String,
}

impl Session {
    pub fn new(id: &str, title: &str) -> Result<Self> {
        Ok(Self {
            name: Self::generate_name(id, title),
        })
    }

    pub fn generate_name(id: &str, title: &str) -> String {
        let safe_title = sanitize_session_name(title);
        format!("{}{}_{}", SESSION_PREFIX, safe_title, truncate_id(id, 8))
    }

    pub fn exists(&self) -> bool {
        if let Some(exists) = session_exists_from_cache(&self.name) {
            return exists;
        }

        Command::new("tmux")
            .args(["has-session", "-t", &self.name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn create(&self, working_dir: &str, command: Option<&str>) -> Result<()> {
        self.create_with_size(working_dir, command, None)
    }

    pub fn create_with_size(
        &self,
        working_dir: &str,
        command: Option<&str>,
        size: Option<(u16, u16)>,
    ) -> Result<()> {
        if self.exists() {
            return Ok(());
        }

        let mut args = build_create_args(&self.name, working_dir, command, size);
        append_remain_on_exit_args(&mut args, &self.name);
        append_store_pane_id_args(&mut args, &self.name);

        let output = Command::new("tmux").args(&args).output()?;

        // Note: With -d flag, tmux new-session returns 0 even if the shell command fails.
        // Log args at debug level for troubleshooting.
        tracing::debug!("tmux new-session args: {:?}", args);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to create tmux session: {}", stderr);
        }

        super::refresh_session_cache();

        Ok(())
    }

    pub fn is_pane_dead(&self) -> bool {
        get_cached_pane_info(&self.name)
            .map(|info| info.is_dead)
            .unwrap_or_else(|| is_pane_dead(&self.name))
    }

    pub fn is_pane_running_shell(&self) -> bool {
        get_cached_pane_info(&self.name)
            .map(|info| super::utils::is_shell_command(&info.current_command))
            .unwrap_or_else(|| is_pane_running_shell(&self.name))
    }

    pub fn kill(&self) -> Result<()> {
        if !self.exists() {
            return Ok(());
        }

        // Kill process trees for ALL panes in the session. This ensures child
        // processes are terminated even for user-created or auto-split panes
        // (e.g. right pane). Agents like Claude spawn subprocesses that may
        // survive tmux's SIGHUP signal.
        for pid in self.all_pane_pids() {
            process::kill_process_tree(pid);
        }

        let output = Command::new("tmux")
            .args(["kill-session", "-t", &self.name])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Session vanished between the exists() check and kill-session
            // (e.g. process tree kill caused tmux to tear it down). That's
            // fine -- the goal was to remove the session and it's gone.
            if !stderr.contains("can't find session") {
                bail!("Failed to kill tmux session: {}", stderr);
            }
        }

        refresh_session_cache();

        Ok(())
    }

    pub fn rename(&self, new_name: &str) -> Result<()> {
        if !self.exists() {
            return Ok(());
        }

        let output = Command::new("tmux")
            .args(["rename-session", "-t", &self.name, new_name])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to rename tmux session: {}", stderr);
        }

        Ok(())
    }

    pub fn attach(&self, profile: &str) -> Result<()> {
        if !self.exists() {
            bail!("Session does not exist: {}", self.name);
        }

        super::utils::setup_session_cycle_bindings(profile);

        let status = Command::new("tmux")
            .args(["attach-session", "-t", &self.name])
            .status()?;

        if !status.success() {
            bail!("Failed to attach to tmux session");
        }

        Ok(())
    }

    pub fn capture_pane(&self, lines: usize) -> Result<String> {
        self.capture_pane_with_size(lines, None, None)
    }

    pub fn capture_pane_cached(&self, lines: usize) -> Result<String> {
        if let Some(content) = get_cached_capture(&self.name, lines, Instant::now()) {
            return Ok(content);
        }

        let content = self.capture_pane(lines)?;
        store_cached_capture(&self.name, content.clone(), lines, Instant::now());
        Ok(content)
    }

    pub fn capture_pane_with_size(
        &self,
        lines: usize,
        _width: Option<u16>,
        _height: Option<u16>,
    ) -> Result<String> {
        if !self.exists() {
            return Ok(String::new());
        }

        // Target the agent pane specifically (via @aoe_agent_pane) so that
        // user-created panes (e.g. from Ctrl+B %) don't interfere with
        // status detection. Falls back to the session name (active pane)
        // when the option is not set.
        let target = get_agent_pane_id(&self.name).unwrap_or_else(|| self.name.clone());

        let output = Command::new("tmux")
            .args([
                "capture-pane",
                "-t",
                &target,
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

    fn all_pane_pids(&self) -> Vec<u32> {
        Command::new("tmux")
            .args(["list-panes", "-t", &self.name, "-F", "#{pane_pid}"])
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| l.trim().parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn pane_count(&self) -> usize {
        Command::new("tmux")
            .args(["list-panes", "-t", &self.name])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0)
    }

    pub fn respawn_agent_pane(&self, command: &str, working_dir: &str) -> Result<()> {
        let target = get_agent_pane_id(&self.name).unwrap_or_else(|| self.name.clone());

        let output = Command::new("tmux")
            .args([
                "respawn-pane",
                "-k",
                "-c",
                working_dir,
                "-t",
                &target,
                command,
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to respawn agent pane: {}", stderr);
        }

        Ok(())
    }

    pub fn send_keys_to_agent_pane(&self, keys: &[&str]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }

        let target = get_agent_pane_id(&self.name).unwrap_or_else(|| self.name.clone());
        let output = Command::new("tmux")
            .arg("send-keys")
            .arg("-t")
            .arg(&target)
            .args(keys)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to send keys to agent pane: {}", stderr);
        }

        Ok(())
    }

    pub fn kill_agent_pane_process_tree(&self) {
        let target = get_agent_pane_id(&self.name).unwrap_or_else(|| self.name.clone());
        if let Some(pid) = process::get_pane_pid(&target) {
            process::kill_process_tree(pid);
        }
    }

    pub fn get_pane_pid(&self) -> Option<u32> {
        if let Some(pid) = get_cached_pane_info(&self.name).and_then(|info| info.pane_pid) {
            return Some(pid);
        }

        let target = get_agent_pane_id(&self.name).unwrap_or_else(|| self.name.clone());
        process::get_pane_pid(&target)
    }

    pub fn get_foreground_pid(&self) -> Option<u32> {
        let pane_pid = self.get_pane_pid()?;
        process::get_foreground_pid(pane_pid).or(Some(pane_pid))
    }

    pub fn detect_status(&self, tool: &str) -> Result<Status> {
        let content = self.capture_pane_cached(50)?;
        let fg_pid = self.get_foreground_pid();
        Ok(super::status_detection::detect_status_from_content(
            &content, tool, fg_pid,
        ))
    }
}

fn get_cached_capture(session_name: &str, lines: usize, now: Instant) -> Option<String> {
    let cache = CAPTURE_CACHE.read().ok()?;
    let entry = cache.data.as_ref()?.get(session_name)?;
    if now.duration_since(entry.timestamp) > CAPTURE_CACHE_TTL || entry.line_count < lines {
        return None;
    }

    Some(entry.content.clone())
}

fn store_cached_capture(session_name: &str, content: String, lines: usize, now: Instant) {
    if let Ok(mut cache) = CAPTURE_CACHE.write() {
        cache.data.get_or_insert_with(HashMap::new).insert(
            session_name.to_string(),
            CaptureCacheEntry {
                content,
                timestamp: now,
                line_count: lines,
            },
        );
    }
}

#[cfg(test)]
fn clear_cached_capture(session_name: &str) {
    if let Ok(mut cache) = CAPTURE_CACHE.write() {
        if let Some(entries) = &mut cache.data {
            entries.remove(session_name);
        }
    }
}

/// Split an existing session's window horizontally and run a command in the new
/// right pane. Sets `remain-on-exit on` on the new pane so it stays visible if
/// the command exits.
pub fn split_window_right(session_name: &str, working_dir: &str, command: &str) -> Result<()> {
    let mut args = vec![
        "split-window".to_string(),
        "-h".to_string(),
        "-t".to_string(),
        session_name.to_string(),
        "-c".to_string(),
        working_dir.to_string(),
        command.to_string(),
    ];

    // Set remain-on-exit on the new (right) pane. After split-window the new
    // pane is the active pane, so we can target it without an explicit pane ID
    // by using the session name (which resolves to the active pane).
    append_remain_on_exit_args(&mut args, session_name);

    // Select the original (left) pane back so that the user lands on the agent pane
    args.extend([
        ";".to_string(),
        "select-pane".to_string(),
        "-t".to_string(),
        format!("{}:.0", session_name),
    ]);

    let output = Command::new("tmux").args(&args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to split window: {}", stderr);
    }
    Ok(())
}

fn sanitize_session_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(20)
        .collect()
}

/// Build the argument list for tmux new-session command.
/// Extracted for testability.
fn build_create_args(
    session_name: &str,
    working_dir: &str,
    command: Option<&str>,
    size: Option<(u16, u16)>,
) -> Vec<String> {
    let mut args = vec![
        "new-session".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        session_name.to_string(),
        "-c".to_string(),
        working_dir.to_string(),
    ];

    if let Some((width, height)) = size {
        args.push("-x".to_string());
        args.push(width.to_string());
        args.push("-y".to_string());
        args.push(height.to_string());
    }

    if let Some(cmd) = command {
        args.push(cmd.to_string());
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: check if tmux is available for tests that need it
    fn tmux_available() -> bool {
        Command::new("tmux")
            .arg("-V")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    #[serial_test::serial]
    fn test_remain_on_exit_and_pane_dead() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }

        let session_name = format!("aoe_test_remain_{}", std::process::id());
        // Chain set-option -p with new-session to avoid race condition
        let output = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                &session_name,
                "-x",
                "80",
                "-y",
                "24",
                "sleep 1",
                ";",
                "set-option",
                "-p",
                "-t",
                &session_name,
                "remain-on-exit",
                "on",
            ])
            .output()
            .expect("tmux new-session");
        assert!(output.status.success());

        // Wait for the sleep command to finish
        std::thread::sleep(std::time::Duration::from_millis(1500));

        // Session should still exist (remain-on-exit keeps it)
        let exists = Command::new("tmux")
            .args(["has-session", "-t", &session_name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(exists, "Session should still exist due to remain-on-exit");

        // Pane should be dead (process exited)
        let pane_dead = Command::new("tmux")
            .args(["display-message", "-t", &session_name, "-p", "#{pane_dead}"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        assert!(pane_dead, "Pane should be dead after command exits");

        // Clean up
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &session_name])
            .output();
    }

    #[test]
    #[serial_test::serial]
    fn test_is_pane_dead_on_running_session() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }

        let session_name = format!("aoe_test_alive_{}", std::process::id());

        // Create a session with a long-running command
        let output = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                &session_name,
                "-x",
                "80",
                "-y",
                "24",
                "sleep 30",
                ";",
                "set-option",
                "-p",
                "-t",
                &session_name,
                "remain-on-exit",
                "on",
            ])
            .output()
            .expect("tmux new-session");
        assert!(output.status.success());

        std::thread::sleep(std::time::Duration::from_millis(200));

        // Pane should NOT be dead (sleep is still running)
        let pane_dead = Command::new("tmux")
            .args(["display-message", "-t", &session_name, "-p", "#{pane_dead}"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        assert!(!pane_dead, "Pane should be alive while command is running");

        // Clean up
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &session_name])
            .output();
    }

    #[test]
    fn test_sanitize_session_name() {
        assert_eq!(sanitize_session_name("my-project"), "my-project");
        assert_eq!(sanitize_session_name("my project"), "my_project");
        assert_eq!(sanitize_session_name("a".repeat(30).as_str()).len(), 20);
    }

    #[test]
    fn test_generate_name() {
        let name = Session::generate_name("abc123def456", "My Project");
        assert!(name.starts_with(SESSION_PREFIX));
        assert!(name.contains("My_Project"));
        assert!(name.contains("abc123de"));
    }

    #[test]
    fn test_build_create_args_without_size() {
        let args = build_create_args("test_session", "/tmp/work", None, None);
        assert_eq!(
            args,
            vec!["new-session", "-d", "-s", "test_session", "-c", "/tmp/work"]
        );
        assert!(!args.contains(&"-x".to_string()));
        assert!(!args.contains(&"-y".to_string()));
    }

    #[test]
    fn test_build_create_args_with_size() {
        let args = build_create_args("test_session", "/tmp/work", None, Some((120, 40)));
        assert!(args.contains(&"-x".to_string()));
        assert!(args.contains(&"120".to_string()));
        assert!(args.contains(&"-y".to_string()));
        assert!(args.contains(&"40".to_string()));

        // Verify order: -x should come before width, -y before height
        let x_idx = args.iter().position(|a| a == "-x").unwrap();
        let y_idx = args.iter().position(|a| a == "-y").unwrap();
        assert_eq!(args[x_idx + 1], "120");
        assert_eq!(args[y_idx + 1], "40");
    }

    #[test]
    fn test_build_create_args_with_command() {
        let args = build_create_args("test_session", "/tmp/work", Some("claude"), None);
        assert_eq!(args.last().unwrap(), "claude");
    }

    #[test]
    fn test_build_create_args_with_size_and_command() {
        let args = build_create_args("test_session", "/tmp/work", Some("claude"), Some((80, 24)));

        // Size args should be present
        assert!(args.contains(&"-x".to_string()));
        assert!(args.contains(&"80".to_string()));
        assert!(args.contains(&"-y".to_string()));
        assert!(args.contains(&"24".to_string()));

        // Command should be last
        assert_eq!(args.last().unwrap(), "claude");
    }

    #[test]
    #[serial_test::serial]
    fn test_is_pane_running_shell_on_shell_session() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }

        let session_name = format!("aoe_test_shell_{}", std::process::id());

        let output = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                &session_name,
                "-x",
                "80",
                "-y",
                "24",
                "sh",
            ])
            .output()
            .expect("tmux new-session");
        assert!(output.status.success());

        std::thread::sleep(std::time::Duration::from_millis(200));

        assert!(
            is_pane_running_shell(&session_name),
            "Session running sh should be detected as a shell"
        );

        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &session_name])
            .output();
    }

    #[test]
    #[serial_test::serial]
    fn test_is_pane_running_shell_on_non_shell_session() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }

        let session_name = format!("aoe_test_noshell_{}", std::process::id());

        let output = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                &session_name,
                "-x",
                "80",
                "-y",
                "24",
                "sleep",
                "30",
            ])
            .output()
            .expect("tmux new-session");
        assert!(output.status.success());

        std::thread::sleep(std::time::Duration::from_millis(200));

        assert!(
            !is_pane_running_shell(&session_name),
            "Session running 'sleep' should not be detected as a shell"
        );

        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &session_name])
            .output();
    }

    #[test]
    fn test_capture_cache_reuses_fresh_entry() {
        let session_name = "aoe_test_capture_cache_reuse";
        clear_cached_capture(session_name);
        let now = Instant::now();
        store_cached_capture(session_name, "cached output".to_string(), 50, now);

        let cached = get_cached_capture(session_name, 20, now + Duration::from_millis(200));

        assert_eq!(cached.as_deref(), Some("cached output"));
        clear_cached_capture(session_name);
    }

    #[test]
    fn test_capture_cache_expires_after_ttl() {
        let session_name = "aoe_test_capture_cache_expired";
        clear_cached_capture(session_name);
        let now = Instant::now();
        store_cached_capture(session_name, "cached output".to_string(), 50, now);

        let cached = get_cached_capture(session_name, 20, now + Duration::from_millis(501));

        assert!(cached.is_none());
        clear_cached_capture(session_name);
    }

    #[test]
    fn test_capture_cache_requires_line_count_upgrade() {
        let session_name = "aoe_test_capture_cache_lines";
        clear_cached_capture(session_name);
        let now = Instant::now();
        store_cached_capture(session_name, "cached output".to_string(), 50, now);

        let cached = get_cached_capture(session_name, 100, now + Duration::from_millis(200));

        assert!(cached.is_none());
        clear_cached_capture(session_name);
    }
}
