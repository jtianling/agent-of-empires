//! tmux session management

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use super::{
    get_cached_pane_info, refresh_session_cache, session_exists_from_cache, tmux_command,
    utils::{
        append_pane_died_hook_args, append_remain_on_exit_args, append_store_pane_id_args,
        append_store_project_path_args, get_agent_pane_id, is_pane_dead, is_pane_running_shell,
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

        crate::tmux::tmux_command()
            .args(["has-session", "-t", &self.name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn create(&self, working_dir: &str, command: Option<&str>) -> Result<()> {
        self.create_with_size(working_dir, command, None, true)
    }

    /// Create the session. `remain_on_exit` keeps the pane alive when its
    /// process exits so the pane-died hook can drop it into a shell; pass
    /// `false` for panes that already run a shell (exit should close them
    /// directly instead of respawning another shell).
    pub fn create_with_size(
        &self,
        working_dir: &str,
        command: Option<&str>,
        size: Option<(u16, u16)>,
        remain_on_exit: bool,
    ) -> Result<()> {
        if self.exists() {
            return Ok(());
        }

        let mut args = build_create_args(&self.name, working_dir, command, size);
        append_remain_on_exit_args(&mut args, &self.name, remain_on_exit);
        append_pane_died_hook_args(&mut args, &self.name);
        append_store_pane_id_args(&mut args, &self.name);
        append_store_project_path_args(&mut args, &self.name, working_dir);

        let output = crate::tmux::tmux_command().args(&args).output()?;

        // Note: With -d flag, tmux new-session returns 0 even if the shell command fails.
        // Log args at debug level for troubleshooting.
        tracing::debug!(
            "tmux new-session args: {:?}",
            crate::tmux::redact_identity_key_args(&args)
        );

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

        let output = crate::tmux::tmux_command()
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

        let output = crate::tmux::tmux_command()
            .args(["rename-session", "-t", &self.name, new_name])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to rename tmux session: {}", stderr);
        }

        Ok(())
    }

    pub fn attach(&self) -> Result<()> {
        if !self.exists() {
            bail!("Session does not exist: {}", self.name);
        }

        let status = crate::tmux::tmux_command()
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

        let output = crate::tmux::tmux_command()
            .args([
                "capture-pane",
                "-t",
                &target,
                "-p",
                "-e",
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
        crate::tmux::tmux_command()
            .args(["list-panes", "-t", &self.name, "-F", "#{pane_pid}"])
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| l.trim().parse::<u32>().ok())
                    // A dead pane reports pane_pid 0; drop 0 (and init pid 1) at
                    // the source so it never reaches kill_process_tree, whose
                    // walk from 0 would target the whole system process tree.
                    .filter(|&pid| !process::is_unsafe_kill_root(pid))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn pane_count(&self) -> usize {
        crate::tmux::tmux_command()
            .args(["list-panes", "-t", &self.name])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0)
    }

    pub fn respawn_agent_pane(
        &self,
        command: &str,
        working_dir: &str,
        remain_on_exit: bool,
    ) -> Result<()> {
        let target = get_agent_pane_id(&self.name).unwrap_or_else(|| self.name.clone());
        respawn_pane_target(&target, command, working_dir, remain_on_exit)
    }

    /// Send a message to the agent, handling multi-line text with Shift+Enter
    /// and pressing Enter to submit.
    pub fn send_keys(&self, text: &str) -> Result<()> {
        if !self.exists() {
            bail!("Session does not exist: {}", self.name);
        }

        let target = format!("{}:^.0", self.name);

        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            Self::tmux_send(&target, &["-l", line])?;
            if i < lines.len() - 1 {
                // ESC + CR: what terminals send for Shift+Enter (inserts newline)
                Self::tmux_send(&target, &["-H", "1b", "0d"])?;
            }
        }

        // Enter to submit
        Self::tmux_send(&target, &["Enter"])?;

        Ok(())
    }

    fn tmux_send(target: &str, args: &[&str]) -> Result<()> {
        let output = crate::tmux::tmux_command()
            .arg("send-keys")
            .args(["-t", target])
            .args(args)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to send keys: {}", stderr);
        }

        Ok(())
    }

    pub fn send_keys_to_agent_pane(&self, keys: &[&str]) -> Result<()> {
        let target = get_agent_pane_id(&self.name).unwrap_or_else(|| self.name.clone());
        send_keys_to_pane_target(&target, keys)
    }

    /// Kill the agent pane's process tree, holding the pane open across the
    /// kill so a respawn has somewhere to land.
    ///
    /// A pane whose `remain-on-exit` is off dies with its process, and a
    /// single-pane session dies with the pane -- so killing first and
    /// respawning after destroys exactly what it meant to restart. Agent panes
    /// are created with `remain-on-exit` on and survive; a shell pane is
    /// created with it off and does not. The subsequent respawn writes the
    /// flag back to whatever the pane should have, so turning it on here is
    /// not left behind.
    ///
    /// If the flag cannot be set, the kill is skipped: the cost of skipping is
    /// orphaned grandchildren, and the cost of proceeding is a destroyed
    /// session.
    pub fn kill_agent_pane_process_tree(&self) {
        let target = get_agent_pane_id(&self.name).unwrap_or_else(|| self.name.clone());
        match set_pane_remain_on_exit(&target, true) {
            Ok(()) => kill_pane_process_tree_target(&target),
            Err(err) => tracing::warn!(
                "Could not hold pane {} open for relaunch, skipping its process-tree kill: {}",
                target,
                err
            ),
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

    /// Capture pane content by tmux pane ID (e.g., "%42").
    pub fn capture_pane_by_id(pane_id: &str, lines: usize) -> Result<String> {
        let output = crate::tmux::tmux_command()
            .args([
                "capture-pane",
                "-t",
                pane_id,
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

/// Respawn an explicit tmux pane target (e.g. `%37`) with `command`, killing
/// the current pane process first (`respawn-pane -k`) and running in `working_dir`.
/// When `remain_on_exit` is set, re-enables remain-on-exit on the pane (the
/// pane-died shell-fallback hook turns it off when it fires); pass `false`
/// for commands that are themselves a shell.
pub fn respawn_pane_target(
    pane: &str,
    command: &str,
    working_dir: &str,
    remain_on_exit: bool,
) -> Result<()> {
    let mut args: Vec<String> = ["respawn-pane", "-k", "-c", working_dir, "-t", pane, command]
        .iter()
        .map(|s| s.to_string())
        .collect();
    append_remain_on_exit_args(&mut args, pane, remain_on_exit);

    let output = crate::tmux::tmux_command().args(&args).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to respawn pane {}: {}", pane, stderr);
    }

    Ok(())
}

/// Kill the process tree rooted at an explicit tmux pane target. Does nothing if
/// the pane has no resolvable pid (already gone).
pub fn kill_pane_process_tree_target(pane: &str) {
    if let Some(pid) = process::get_pane_pid(pane) {
        process::kill_process_tree(pid);
    }
}

/// Set `remain-on-exit` on one pane on its own.
///
/// Killing a pane's process from outside tmux only leaves the pane behind when
/// remain-on-exit is on; with it off tmux destroys the pane the moment the
/// process goes, and anything that meant to respawn into that pane has nothing
/// left to target. A caller that kills a pane in order to relaunch it therefore
/// has to establish that itself rather than assume how the pane was created.
///
/// The result is reported rather than swallowed precisely because callers rely
/// on it as a precondition: a caller that kills the pane's process anyway after
/// this failed would destroy the pane it meant to protect.
pub fn set_pane_remain_on_exit(pane: &str, on: bool) -> Result<()> {
    let output = crate::tmux::tmux_command()
        .args(super::utils::remain_on_exit_args(pane, on))
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Failed to set remain-on-exit={} on pane {}: {}",
            if on { "on" } else { "off" },
            pane,
            stderr.trim()
        );
    }

    Ok(())
}

/// Capture only what is currently on a pane's screen, by explicit pane target.
///
/// Deliberately excludes scrollback. A caller that decides whether to send input
/// based on what a pane shows must see the pane's present state: history that
/// merely mentions a prompt is not that pane asking a question now, and acting
/// on it means typing into whatever the pane is actually doing.
///
/// Failure is returned rather than flattened into an empty capture, so a caller
/// can tell "this pane shows nothing I act on" from "I could not read this pane".
///
/// `-J` rejoins the lines tmux itself soft-wrapped, which split mid-word and so
/// cannot be repaired after the fact. It does not address text an application
/// re-flowed to the pane width on its own -- those arrive as genuinely separate
/// lines, broken at spaces -- so a caller matching phrases still has to tolerate
/// that. The two kinds of wrapping need different remedies and both occur.
pub fn capture_pane_screen(pane: &str) -> Result<String> {
    let output = crate::tmux::tmux_command()
        .args(["capture-pane", "-t", pane, "-p", "-e", "-J"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to capture pane {}: {}", pane, stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Send raw key strings to an explicit tmux pane target. No-op for empty input.
pub fn send_keys_to_pane_target(pane: &str, keys: &[&str]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }

    let output = crate::tmux::tmux_command()
        .arg("send-keys")
        .arg("-t")
        .arg(pane)
        .args(keys)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to send keys to pane {}: {}", pane, stderr);
    }

    Ok(())
}

/// Split an existing session's window horizontally and run a command in the new
/// right pane, returning that pane's id. When `remain_on_exit` is set, the new
/// pane stays alive if the command exits (letting the pane-died hook drop it
/// into a shell); pass `false` for panes that already run a shell.
///
/// The id is reported by `split-window` itself rather than read back
/// afterwards: the caller records the pane's durable slot, and by the time it
/// could ask the session which pane is active the user may have moved.
pub fn split_window_right(
    session_name: &str,
    working_dir: &str,
    command: &str,
    remain_on_exit: bool,
) -> Result<String> {
    let primary_target =
        get_agent_pane_id(session_name).unwrap_or_else(|| format!("{}:.0", session_name));
    let args = vec![
        "split-window".to_string(),
        "-h".to_string(),
        "-P".to_string(),
        "-F".to_string(),
        "#{pane_id}".to_string(),
        "-t".to_string(),
        session_name.to_string(),
        "-c".to_string(),
        working_dir.to_string(),
        command.to_string(),
    ];

    tracing::debug!(
        session = session_name,
        working_dir,
        command = %crate::tmux::redact_identity_key(command),
        args = ?crate::tmux::redact_identity_key_args(&args),
        "Splitting tmux window for right pane"
    );

    let output = crate::tmux::tmux_command().args(&args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to split window: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pane_id = match stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(pane_id) => pane_id.to_string(),
        None => bail!("split-window did not report a pane id"),
    };

    if let Err(error) = set_pane_remain_on_exit(&pane_id, remain_on_exit) {
        return Err(match kill_pane_exact(&pane_id) {
            Ok(()) => error.context(format!("configuring pane {pane_id} after split")),
            Err(rollback_error) => {
                anyhow::anyhow!("{error:#}. Failed to roll back pane {pane_id}: {rollback_error:#}")
            }
        });
    }

    let selected = match crate::tmux::tmux_command()
        .args(["select-pane", "-t", &primary_target])
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return Err(match kill_pane_exact(&pane_id) {
                Ok(()) => error.into(),
                Err(rollback_error) => anyhow::anyhow!(
                    "{error}. Failed to roll back pane {pane_id}: {rollback_error:#}"
                ),
            });
        }
    };
    if !selected.status.success() {
        let error = anyhow::anyhow!(
            "Failed to select primary pane after split: {}",
            String::from_utf8_lossy(&selected.stderr).trim()
        );
        return Err(match kill_pane_exact(&pane_id) {
            Ok(()) => error,
            Err(rollback_error) => {
                anyhow::anyhow!("{error:#}. Failed to roll back pane {pane_id}: {rollback_error:#}")
            }
        });
    }

    Ok(pane_id)
}

/// Kill one pane by the exact id returned from `split-window -P`.
pub fn kill_pane_exact(pane_id: &str) -> Result<()> {
    let valid = pane_id
        .strip_prefix('%')
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()));
    if !valid {
        anyhow::bail!("invalid tmux pane id: {pane_id}");
    }
    let output = tmux_command()
        .args(["kill-pane", "-t", pane_id])
        .output()
        .context("failed to kill rolled-back pane")?;
    if !output.status.success() {
        anyhow::bail!(
            "failed to kill pane {pane_id}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Split the target pane horizontally and return the new pane's id.
///
/// Cold-start recovery chains each split from the pane created immediately
/// before it. tmux assigns custom-layout leaves in window pane-list order, so
/// this preserves durable slot order when the saved layout is applied.
pub fn split_window_right_capture_pane(
    target_pane: &str,
    working_dir: &str,
    command: &str,
    remain_on_exit: bool,
) -> Result<String> {
    let args = vec![
        "split-window".to_string(),
        "-h".to_string(),
        "-P".to_string(),
        "-F".to_string(),
        "#{pane_id}".to_string(),
        "-t".to_string(),
        target_pane.to_string(),
        "-c".to_string(),
        working_dir.to_string(),
        command.to_string(),
    ];

    let output = crate::tmux::tmux_command().args(&args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to split window: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pane_id = match stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(pane_id) => pane_id.to_string(),
        None => bail!("split-window did not report a pane id"),
    };

    // Written to the pane the split produced, not to the one it was split from.
    // The option cannot ride along on the `split-window` command itself: its
    // only pane target there is the split source, so setting it in the same
    // invocation writes the new pane's setting onto the old pane.
    set_pane_remain_on_exit(&pane_id, remain_on_exit)?;

    Ok(pane_id)
}

/// Read the active window's serialized layout for one session.
pub fn session_window_layout(session_name: &str) -> Result<String> {
    let output = crate::tmux::tmux_command()
        .args([
            "display-message",
            "-p",
            "-t",
            session_name,
            "#{window_layout}",
        ])
        .output()?;
    if !output.status.success() {
        bail!(
            "Failed to read window layout: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let layout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if layout.is_empty() {
        bail!("tmux returned an empty window layout");
    }
    Ok(layout)
}

/// Apply a serialized layout to the session's active window. The layout is
/// passed as one argv value, never through a shell.
pub fn apply_window_layout(session_name: &str, layout: &str) -> Result<()> {
    let output = crate::tmux::tmux_command()
        .args(["select-layout", "-t", session_name, layout])
        .output()?;
    if !output.status.success() {
        bail!(
            "Failed to apply window layout: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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

    /// Kills one session by its exact name when dropped.
    ///
    /// A test that creates a tmux session and cleans it up on the last line
    /// cleans up only when it passes. Every assertion between is an early exit
    /// that leaves the server, its shell, and that shell's children running --
    /// and a session name derived from the pid can then collide with a later
    /// test once the pid is reused. Unwinding is the path that needs the
    /// cleanup most, so the cleanup belongs to the value's lifetime.
    ///
    /// Exact name only: never a pattern, never `kill-server`. See AGENTS.md
    /// "Tmux Session Safety".
    struct SessionGuard(String);

    impl Drop for SessionGuard {
        fn drop(&mut self) {
            let _ = crate::tmux::tmux_command()
                .args(["kill-session", "-t", &self.0])
                .output();
        }
    }

    /// Helper: check if tmux is available for tests that need it
    fn tmux_available() -> bool {
        crate::tmux::tmux_command()
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
        crate::tmux::isolate_tmux_socket();

        let session_name = format!("aoe_test_remain_{}", std::process::id());
        // Chain set-option -p with new-session to avoid race condition
        let output = crate::tmux::tmux_command()
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
        let exists = crate::tmux::tmux_command()
            .args(["has-session", "-t", &session_name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(exists, "Session should still exist due to remain-on-exit");

        // Pane should be dead (process exited)
        let pane_dead = crate::tmux::tmux_command()
            .args(["display-message", "-t", &session_name, "-p", "#{pane_dead}"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        assert!(pane_dead, "Pane should be dead after command exits");

        // Clean up
        let _ = crate::tmux::tmux_command()
            .args(["kill-session", "-t", &session_name])
            .output();
    }

    #[test]
    #[serial_test::serial]
    fn test_pane_died_hook_drops_pane_into_shell() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }
        crate::tmux::isolate_tmux_socket();

        let id = format!("deadfall{}", std::process::id());
        let session = Session::new(&id, "test_deadfall").expect("session");
        session
            .create("/tmp", Some("sleep 1"))
            .expect("create session");
        let name = Session::generate_name(&id, "test_deadfall");

        // After the command exits, the pane-died hook should respawn the pane
        // into the user's shell instead of leaving a dead pane.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut in_shell = false;
        while std::time::Instant::now() < deadline {
            if crate::tmux::utils::is_pane_running_shell(&name) {
                in_shell = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(in_shell, "pane should fall back to a shell after exit");
        assert!(
            !crate::tmux::utils::is_pane_dead(&name),
            "pane should not stay dead after the fallback hook fires"
        );

        // Exiting the fallback shell should close the pane and end the
        // session (the hook turned remain-on-exit off).
        let _ = crate::tmux::tmux_command()
            .args(["send-keys", "-t", &name, "exit", "Enter"])
            .output();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut gone = false;
        while std::time::Instant::now() < deadline {
            let exists = crate::tmux::tmux_command()
                .args(["has-session", "-t", &name])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !exists {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(gone, "session should end when the fallback shell exits");

        let _ = crate::tmux::tmux_command()
            .args(["kill-session", "-t", &name])
            .output();
    }

    #[test]
    #[serial_test::serial]
    fn test_shell_pane_closes_on_single_exit() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }
        crate::tmux::isolate_tmux_socket();

        let id = format!("shellexit{}", std::process::id());
        let session = Session::new(&id, "test_shellexit").expect("session");
        // Shell sessions are created without remain-on-exit: a single exit
        // must close the pane (and session) with no fallback-shell respawn.
        session
            .create_with_size("/tmp", Some("sh"), Some((80, 24)), false)
            .expect("create session");
        let name = Session::generate_name(&id, "test_shellexit");

        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = crate::tmux::tmux_command()
            .args(["send-keys", "-t", &name, "exit", "Enter"])
            .output();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut gone = false;
        while std::time::Instant::now() < deadline {
            let exists = crate::tmux::tmux_command()
                .args(["has-session", "-t", &name])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !exists {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(gone, "shell session should end after a single exit");

        let _ = crate::tmux::tmux_command()
            .args(["kill-session", "-t", &name])
            .output();
    }

    #[test]
    #[serial_test::serial]
    fn test_is_pane_dead_on_running_session() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }
        crate::tmux::isolate_tmux_socket();

        let session_name = format!("aoe_test_alive_{}", std::process::id());

        // Create a session with a long-running command
        let output = crate::tmux::tmux_command()
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
        let pane_dead = crate::tmux::tmux_command()
            .args(["display-message", "-t", &session_name, "-p", "#{pane_dead}"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        assert!(!pane_dead, "Pane should be alive while command is running");

        // Clean up
        let _ = crate::tmux::tmux_command()
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
        crate::tmux::isolate_tmux_socket();

        let session_name = format!("aoe_test_shell_{}", std::process::id());

        let output = crate::tmux::tmux_command()
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

        let _ = crate::tmux::tmux_command()
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
        crate::tmux::isolate_tmux_socket();

        let session_name = format!("aoe_test_noshell_{}", std::process::id());

        let output = crate::tmux::tmux_command()
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

        let _ = crate::tmux::tmux_command()
            .args(["kill-session", "-t", &session_name])
            .output();
    }

    #[test]
    #[serial_test::serial]
    fn test_respawn_pane_target_respawns_given_pane() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }
        crate::tmux::isolate_tmux_socket();

        let session_name = format!("aoe_test_respawn_target_{}", std::process::id());
        let output = crate::tmux::tmux_command()
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
            ])
            .output()
            .expect("tmux new-session");
        assert!(output.status.success());

        let pane_id = crate::tmux::tmux_command()
            .args(["display-message", "-t", &session_name, "-p", "#{pane_id}"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .expect("pane id");

        respawn_pane_target(&pane_id, "sleep 99", "/tmp", true).expect("respawn target");

        // respawn_pane_target must re-enable remain-on-exit (the pane-died
        // hook turns it off when it fires).
        let remain = crate::tmux::tmux_command()
            .args(["show-options", "-p", "-t", &pane_id, "remain-on-exit"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        assert!(
            remain.contains("on"),
            "respawned pane should have remain-on-exit on, got {:?}",
            remain
        );

        let start_cmd = crate::tmux::tmux_command()
            .args([
                "display-message",
                "-t",
                &pane_id,
                "-p",
                "#{pane_start_command}",
            ])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        assert!(
            start_cmd.contains("sleep 99"),
            "respawned pane should run the new command, got {:?}",
            start_cmd
        );

        let _ = crate::tmux::tmux_command()
            .args(["kill-session", "-t", &session_name])
            .output();
    }

    #[test]
    #[serial_test::serial]
    fn test_send_keys_to_pane_target_empty_is_noop() {
        // Empty key list returns Ok without invoking tmux against a real pane.
        assert!(send_keys_to_pane_target("%nonexistent", &[]).is_ok());
    }

    /// A pane's screen is not its history. Deciding whether to send input from a
    /// capture that includes scrollback means a pane that once printed a prompt
    /// looks like a pane asking one now -- and the keystroke goes to whatever it
    /// is really doing.
    #[test]
    #[serial_test::serial]
    fn test_capture_pane_screen_excludes_scrollback() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }
        crate::tmux::isolate_tmux_socket();

        let session_name = format!("aoe_test_capture_screen_{}", std::process::id());
        let marker = "I am using this for local development";
        // `sh` with no input can reach EOF and exit, taking the session with it
        // and leaving every later step to fail on a server that is no longer
        // there. A shell that stays put keeps the failure modes about capture.
        let created = crate::tmux::tmux_command()
            .args([
                "new-session",
                "-d",
                "-s",
                &session_name,
                "-x",
                "80",
                "-y",
                "10",
                "sh -c 'while :; do sleep 60; done'",
            ])
            .output()
            .expect("tmux new-session");
        assert!(
            created.status.success(),
            "new-session failed: {}",
            String::from_utf8_lossy(&created.stderr)
        );
        // From here on every step can fail; the guard owns the teardown so an
        // early exit does not leave the server and its endless shell behind.
        let _guard = SessionGuard(session_name.clone());

        // `new-session` returning does not mean the server is answering yet: a
        // first query can still land before the socket is connectable, which
        // fails as "error connecting ... No such file or directory" and then
        // reads an empty stdout as a pane id. Wait for the session to answer.
        let deadline = Instant::now() + Duration::from_secs(5);
        let pane = loop {
            let out = crate::tmux::tmux_command()
                .args(["display-message", "-t", &session_name, "-p", "#{pane_id}"])
                .output()
                .expect("tmux display-message");
            let pane = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if out.status.success() && pane.starts_with('%') {
                break pane;
            }
            assert!(
                Instant::now() < deadline,
                "session never answered: status={:?} stdout={:?} stderr={:?}",
                out.status,
                pane,
                String::from_utf8_lossy(&out.stderr)
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        // Put the marker in history, then push it off a 10-row screen.
        send_keys_to_pane_target(&pane, &[&format!("echo '{marker}'"), "Enter"])
            .expect("send marker");
        for _ in 0..40 {
            send_keys_to_pane_target(&pane, &["echo filler", "Enter"]).expect("send filler");
        }
        std::thread::sleep(Duration::from_millis(600));

        let screen = capture_pane_screen(&pane).expect("capture screen");
        assert!(
            !screen.contains(marker),
            "the visible screen must not carry scrolled-off history, got {screen:?}"
        );

        let with_history = crate::tmux::tmux_command()
            .args(["capture-pane", "-t", &pane, "-p", "-S", "-200"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        assert!(
            with_history.contains(marker),
            "precondition: the marker must really be in this pane's scrollback"
        );

        // Normal path: tear down explicitly and check it worked. The guard is the
        // backstop for the paths that never reach this line.
        let killed = crate::tmux::tmux_command()
            .args(["kill-session", "-t", &session_name])
            .output()
            .expect("kill-session");
        assert!(
            killed.status.success(),
            "cleanup failed: {}",
            String::from_utf8_lossy(&killed.stderr)
        );
    }

    /// The split's `remain_on_exit` describes the pane the split produced. The
    /// only pane `split-window` can target is the pane being split, so setting
    /// the option in that same invocation writes the new pane's value onto the
    /// old one -- silently, and in the direction that matters: it turns the
    /// source pane's protection off.
    #[test]
    #[serial_test::serial]
    fn test_split_capture_pane_sets_remain_on_exit_on_the_new_pane_only() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }
        crate::tmux::isolate_tmux_socket();

        let session_name = format!("aoe_test_split_remain_{}", std::process::id());
        let created = crate::tmux::tmux_command()
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
            ])
            .output()
            .expect("tmux new-session");
        assert!(created.status.success());
        let _guard = SessionGuard(session_name.clone());

        let source_pane = crate::tmux::tmux_command()
            .args(["display-message", "-t", &session_name, "-p", "#{pane_id}"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .expect("pane id");

        // The source pane is protected, as a pane awaiting relaunch is.
        set_pane_remain_on_exit(&source_pane, true).expect("hold source pane open");

        let new_pane = split_window_right_capture_pane(&source_pane, "/tmp", "sleep 30", false)
            .expect("split window");
        assert_ne!(new_pane, source_pane);

        let read = |pane: &str| {
            crate::tmux::tmux_command()
                .args(["show-options", "-p", "-t", pane, "remain-on-exit"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default()
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string()
        };

        assert_eq!(
            read(&new_pane),
            "off",
            "the new pane takes the split's value"
        );
        assert_eq!(
            read(&source_pane),
            "on",
            "the source pane keeps its own setting: writing the split's value \
             onto it would drop the protection a pending relaunch depends on"
        );

        // Normal path: tear down explicitly and check it worked. The guard is the
        // backstop for the paths that never reach this line.
        let killed = crate::tmux::tmux_command()
            .args(["kill-session", "-t", &session_name])
            .output()
            .expect("kill-session");
        assert!(
            killed.status.success(),
            "cleanup failed: {}",
            String::from_utf8_lossy(&killed.stderr)
        );
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
