//! Process utilities for tmux session management

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

/// Get the PID of the shell process running in a tmux pane.
/// `target` can be a session name, a pane ID (e.g. `%42`), or any valid tmux target.
pub fn get_pane_pid(target: &str) -> Option<u32> {
    let output = crate::tmux::tmux_command()
        .args(["display-message", "-t", target, "-p", "#{pane_pid}"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // A dead pane (the command exited but `remain-on-exit` keeps it visible)
    // reports `#{pane_pid}` as 0. Treat that as "no process": returning Some(0)
    // here would feed pid 0 into kill_process_tree, whose descendant walk from 0
    // covers the ENTIRE system process tree. Filter out 0 (and the init pid 1).
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()
        .filter(|&pid| pid > 1)
}

/// Get the foreground process group leader PID for a given shell PID
/// This finds the actual process that has the terminal foreground
pub fn get_foreground_pid(shell_pid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        linux::get_foreground_pid(shell_pid)
    }

    #[cfg(target_os = "macos")]
    {
        macos::get_foreground_pid(shell_pid)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = shell_pid;
        None
    }
}

/// Get the comm name (binary name) of a process by PID
pub fn get_process_comm(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux::get_process_comm(pid)
    }

    #[cfg(target_os = "macos")]
    {
        macos::get_process_comm(pid)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// A pid that must never be used as the root of a kill: 0 is the kernel /
/// dead-pane sentinel and 1 is init/launchd. Walking descendants from either
/// would enumerate (and SIGTERM/SIGKILL) the entire system process tree.
fn is_unsafe_kill_root(pid: u32) -> bool {
    pid <= 1
}

/// Kill a process and all its descendants
/// Sends SIGTERM first, then SIGKILL to any survivors
pub fn kill_process_tree(pid: u32) {
    // Hard guard against catastrophic kills. A dead tmux pane resolves to pane
    // pid 0; without this, kill_process_tree(0) would tear down every process on
    // the machine (including this one). Callers should already avoid pid <= 1,
    // but this backstop makes the invariant unconditional.
    if is_unsafe_kill_root(pid) {
        return;
    }

    #[cfg(target_os = "linux")]
    {
        linux::kill_process_tree(pid);
    }

    #[cfg(target_os = "macos")]
    {
        macos::kill_process_tree(pid);
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        // No-op on unsupported platforms, fall back to tmux kill-session only
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_unsafe_kill_root() {
        // 0 (kernel / dead-pane sentinel) and 1 (init/launchd) must be rejected
        // so kill_process_tree never walks the whole system tree from them.
        assert!(is_unsafe_kill_root(0));
        assert!(is_unsafe_kill_root(1));
        assert!(!is_unsafe_kill_root(2));
        assert!(!is_unsafe_kill_root(99999));
    }
}
