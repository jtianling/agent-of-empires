//! Process utilities for tmux session management

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

pub(crate) fn parent_id() -> u32 {
    nix::unistd::getppid().as_raw() as u32
}

pub(crate) async fn output_with_timeout(
    command: std::process::Command,
    timeout: std::time::Duration,
) -> anyhow::Result<std::process::Output> {
    use anyhow::Context;

    let program = command.get_program().to_string_lossy().into_owned();
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    tokio::time::timeout(timeout, command.output())
        .await
        .with_context(|| format!("Timed out running {program}"))?
        .with_context(|| format!("Running {program}"))
}

pub(crate) fn arguments(pid: u32) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(pid > 0 && pid <= i32::MAX as u32, "Invalid process ID");
    #[cfg(target_os = "linux")]
    {
        linux::arguments(pid)
    }
    #[cfg(target_os = "macos")]
    {
        macos::arguments(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("Process arguments are unsupported on this platform")
    }
}

pub(crate) fn start_identity(pid: u32) -> anyhow::Result<Option<String>> {
    anyhow::ensure!(pid > 0 && pid <= i32::MAX as u32, "Invalid process ID");
    #[cfg(target_os = "linux")]
    {
        linux::start_identity(pid)
    }
    #[cfg(target_os = "macos")]
    {
        macos::start_identity(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("Process start identity is unsupported on this platform")
    }
}

pub(crate) fn isolate_process_group(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

pub(crate) fn group_members(group: u32) -> anyhow::Result<Vec<(u32, String)>> {
    use anyhow::{ensure, Context};
    ensure!(group > 1, "Invalid owned process group");
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,pgid="])
        .output()
        .context("Inspecting owned process group")?;
    ensure!(
        output.status.success(),
        "Inspecting process group failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut members = Vec::new();
    for line in String::from_utf8(output.stdout)?.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        ensure!(fields.len() == 2, "Invalid process group response");
        let pid = fields[0].parse::<u32>()?;
        if fields[1].parse::<u32>()? == group {
            if let Some(identity) = start_identity(pid)? {
                members.push((pid, identity));
            }
        }
    }
    Ok(members)
}

pub(crate) fn signal_verified_group(
    group: u32,
    witnesses: &[(u32, String)],
    force: bool,
) -> anyhow::Result<()> {
    use anyhow::{ensure, Context};
    use nix::errno::Errno;
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    let current = group_members(group)?;
    if current.is_empty() {
        return Ok(());
    }
    ensure!(
        current.iter().any(|member| witnesses.contains(member)),
        "Owned process group has no matching start identity"
    );
    let signal = if force {
        Signal::SIGKILL
    } else {
        Signal::SIGTERM
    };
    match killpg(Pid::from_raw(i32::try_from(group)?), signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(error).context("Signalling verified owned process group"),
    }
}

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
    // covers the ENTIRE system process tree. Reject 0 (and the init pid 1).
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()
        .filter(|&pid| !is_unsafe_kill_root(pid))
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
///
/// The single source of truth for this invariant. Used both as the
/// `kill_process_tree` backstop and to filter pids at their source (a dead tmux
/// pane reports `pane_pid` 0), so a bad pid is dropped before it ever reaches a
/// kill rather than relying on the backstop alone.
pub fn is_unsafe_kill_root(pid: u32) -> bool {
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
