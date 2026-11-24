//! Linux-specific process utilities

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{ensure, Context};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use tracing::debug;

pub(super) fn arguments(pid: u32) -> anyhow::Result<Vec<String>> {
    read_arguments(Path::new("/proc"), pid)
}

fn read_arguments(proc: &Path, pid: u32) -> anyhow::Result<Vec<String>> {
    let bytes = fs::read(proc.join(pid.to_string()).join("cmdline"))
        .with_context(|| format!("Reading arguments of PID {pid}"))?;
    ensure!(bytes.last() == Some(&0), "Invalid process arguments");
    Ok(bytes[..bytes.len() - 1]
        .split(|byte| *byte == 0)
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect())
}

/// Kill a process and all its descendants
/// Uses SIGTERM first, then SIGKILL after a short delay for stragglers
pub fn kill_process_tree(pid: u32) {
    kill_process_tree_gracefully(pid, std::time::Duration::from_millis(100));
}

/// Kill a process and all its descendants, waiting up to `grace` for clean
/// SIGTERM exit before escalating to SIGKILL. Returns as soon as all targeted
/// processes have exited.
pub fn kill_process_tree_gracefully(pid: u32, grace: std::time::Duration) {
    let mut pids_to_kill = vec![pid];
    collect_descendants(pid, &mut pids_to_kill);

    debug!(
        pid,
        descendants = ?pids_to_kill,
        "Killing process tree"
    );

    for &p in pids_to_kill.iter().rev() {
        let _ = kill(Pid::from_raw(p as i32), Signal::SIGTERM);
    }

    super::wait_for_exits(&pids_to_kill, grace, process_exists);

    let mut escalated = false;
    for &p in pids_to_kill.iter().rev() {
        if process_exists(p) {
            debug!(pid = p, "Process survived SIGTERM, sending SIGKILL");
            let _ = kill(Pid::from_raw(p as i32), Signal::SIGKILL);
            escalated = true;
        }
    }
    if escalated {
        super::wait_for_exits(
            &pids_to_kill,
            std::time::Duration::from_millis(500),
            process_exists,
        );
    }
}

/// Recursively collect all descendant PIDs of a process
fn collect_descendants(pid: u32, pids: &mut Vec<u32>) {
    let proc_dir = Path::new("/proc");
    if !proc_dir.exists() {
        return;
    }

    let Ok(entries) = fs::read_dir(proc_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip non-numeric entries
        let Ok(child_pid) = name_str.parse::<u32>() else {
            continue;
        };

        // Read the process's parent PID
        let stat_path = entry.path().join("stat");
        let Ok(content) = fs::read_to_string(&stat_path) else {
            continue;
        };

        if let Some(ppid) = parse_stat_field(&content, 3) {
            if ppid as u32 == pid {
                pids.push(child_pid);
                // Recurse to find grandchildren
                collect_descendants(child_pid, pids);
            }
        }
    }
}

/// Check if a process still exists
fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{}", pid)).exists()
}

/// Get the foreground process group leader for a shell PID
/// Walks the process tree to find the actual foreground process
pub fn get_foreground_pid(shell_pid: u32) -> Option<u32> {
    // Read the shell's stat to get its controlling terminal
    let stat_path = format!("/proc/{}/stat", shell_pid);
    let stat_content = fs::read_to_string(&stat_path).ok()?;

    // Parse stat: pid (comm) state ppid pgrp session tty_nr tpgid ...
    // tpgid (field 8, 0-indexed 7) is the foreground process group ID
    let tpgid = parse_stat_field(&stat_content, 7)?;

    if tpgid <= 0 {
        return Some(shell_pid);
    }

    // Find a process in the foreground process group
    // The tpgid is a process group ID, we need to find a process in that group
    find_process_in_group(tpgid as u32).or(Some(shell_pid))
}

/// Find a process that belongs to the given process group
fn find_process_in_group(pgrp: u32) -> Option<u32> {
    let proc_dir = Path::new("/proc");
    if !proc_dir.exists() {
        return None;
    }

    for entry in fs::read_dir(proc_dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip non-numeric entries
        if !name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let pid: u32 = name_str.parse().ok()?;
        let stat_path = entry.path().join("stat");

        if let Ok(content) = fs::read_to_string(&stat_path) {
            // Field 5 (0-indexed 4) is the process group ID
            if let Some(proc_pgrp) = parse_stat_field(&content, 4) {
                if proc_pgrp as u32 == pgrp {
                    return Some(pid);
                }
            }
        }
    }

    None
}

/// Parse a specific field from /proc/[pid]/stat
/// Fields are space-separated but comm (field 2) can contain spaces and is in parens
fn parse_stat_field(content: &str, field_idx: usize) -> Option<i64> {
    // Find the closing paren of comm field, then parse from there
    let close_paren = content.rfind(')')?;
    let after_comm = &content[close_paren + 2..]; // Skip ") "

    // Fields after comm start at index 2 (state is index 2)
    // So field_idx 4 means we want the 3rd field after comm (index 2 in after_comm split)
    let adjusted_idx = field_idx.checked_sub(2)?;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    fields.get(adjusted_idx)?.parse().ok()
}

/// Get the comm name (binary name) of a process by PID
pub fn get_process_comm(pid: u32) -> Option<String> {
    let comm = fs::read_to_string(format!("/proc/{}/comm", pid)).ok()?;
    let trimmed = comm.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn ticks_per_second() -> anyhow::Result<u64> {
    let hz = unsafe { nix::libc::sysconf(nix::libc::_SC_CLK_TCK) };
    ensure!(hz > 0, "Invalid process clock frequency");
    Ok(hz as u64)
}

pub(crate) fn pane_sample_time() -> anyhow::Result<u64> {
    let mut time = std::mem::MaybeUninit::uninit();
    if unsafe { nix::libc::clock_gettime(nix::libc::CLOCK_BOOTTIME, time.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).context("Reading boot clock");
    }
    let time = unsafe { time.assume_init() };
    let hz = ticks_per_second()?;
    Ok(u64::try_from(time.tv_sec)? * hz + u64::try_from(time.tv_nsec)? * hz / 1_000_000_000)
}

pub(crate) fn pane_processes(
    root: u32,
    sampled_before: u64,
) -> anyhow::Result<Vec<super::PaneProcess>> {
    let proc = Path::new("/proc");
    let mut fallback = None;
    super::pane::collect(
        root,
        sampled_before,
        |pid| process_info(proc, pid),
        arguments,
        |pid| child_pids(proc, pid, &mut fallback),
    )
}

fn child_pids(
    proc: &Path,
    pid: u32,
    fallback: &mut Option<HashMap<u32, Vec<u32>>>,
) -> anyhow::Result<Vec<u32>> {
    if fallback.is_none() {
        match task_children(&proc.join(pid.to_string()).join("task")) {
            Ok(children) => return Ok(children),
            Err(error)
                if error.downcast_ref::<std::io::Error>().is_some_and(|e| {
                    matches!(
                        e.raw_os_error(),
                        Some(
                            nix::libc::ENOENT
                                | nix::libc::ESRCH
                                | nix::libc::EACCES
                                | nix::libc::EPERM
                        )
                    )
                }) => {}
            Err(error) => return Err(error),
        }
        let mut parents = HashMap::<u32, Vec<u32>>::new();
        for entry in fs::read_dir(proc)? {
            let Ok(child) = entry?.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            if let Some((parent, _)) = process_info(proc, child)? {
                parents.entry(parent).or_default().push(child);
            }
        }
        *fallback = Some(parents);
    }
    Ok(fallback
        .as_mut()
        .and_then(|map| map.remove(&pid))
        .unwrap_or_default())
}

fn task_children(tasks: &Path) -> anyhow::Result<Vec<u32>> {
    let mut children = Vec::new();
    for task in fs::read_dir(tasks)? {
        let path = task?.path().join("children");
        for child in fs::read_to_string(&path)
            .with_context(|| format!("Reading {}", path.display()))?
            .split_whitespace()
        {
            children.push(child.parse()?);
        }
    }
    Ok(children)
}

pub(crate) fn start_unix(start: super::pane::Start) -> anyhow::Result<i64> {
    let hz = ticks_per_second()?;
    let boot = fs::read_to_string("/proc/stat").context("Reading system boot time")?;
    let boot = boot
        .lines()
        .find_map(|line| line.strip_prefix("btime "))
        .context("Missing system boot time")?
        .parse::<i64>()?;
    boot.checked_add(i64::try_from(start / hz)?)
        .context("Invalid process start time")
}

fn process_info(proc: &Path, pid: u32) -> anyhow::Result<Option<(u32, super::pane::Start)>> {
    let stat = match fs::read(proc.join(pid.to_string()).join("stat")) {
        Ok(stat) => stat,
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(nix::libc::ENOENT | nix::libc::ESRCH)
            ) =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error).context("Reading process start identity"),
    };
    let stat = String::from_utf8_lossy(&stat);
    let fields = stat.rsplit_once(')').context("Invalid process stat")?.1;
    if fields.split_whitespace().next() == Some("Z") {
        return Ok(None);
    }
    let ticks = fields
        .split_whitespace()
        .nth(19)
        .context("Missing process start time")?
        .parse::<u64>()?;
    let parent = fields
        .split_whitespace()
        .nth(1)
        .context("Missing parent PID")?
        .parse()?;
    Ok(Some((parent, ticks)))
}

pub(super) fn start_identity(pid: u32) -> anyhow::Result<Option<String>> {
    let Some((_, ticks)) = process_info(Path::new("/proc"), pid)? else {
        return Ok(None);
    };
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    Ok(Some(format!("linux:{}:{ticks}", boot.trim())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_thread_children_falls_back_to_stat_without_reading_unrelated_argv() {
        let dir = tempfile::tempdir().unwrap();
        let proc = dir.path();
        for (pid, parent) in [(10, 1), (11, 10), (12, 2)] {
            let path = proc.join(pid.to_string());
            fs::create_dir_all(path.join("task").join(pid.to_string())).unwrap();
            fs::write(
                path.join("stat"),
                format!("{pid} (name) S {parent} {} 9", "0 ".repeat(17)),
            )
            .unwrap();
        }
        fs::write(proc.join("10/cmdline"), b"shell\0").unwrap();
        fs::write(proc.join("11/cmdline"), b"helper\0bad\xff\0").unwrap();
        let mut fallback = None;
        let tree = super::super::pane::collect(
            10,
            15,
            |pid| process_info(proc, pid),
            |pid| read_arguments(proc, pid),
            |pid| child_pids(proc, pid, &mut fallback),
        )
        .unwrap();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[1].pid, 11);
        assert_eq!(tree[1].args, ["helper", "bad�"]);
    }

    #[test]
    fn test_parse_stat_field() {
        // Example stat line (simplified)
        let stat = "1234 (bash) S 1233 1234 1234 34816 1234 4194304 1234 0 0 0";
        // Fields: pid(0) comm(1) state(2) ppid(3) pgrp(4) session(5) tty(6) tpgid(7) ...

        assert_eq!(parse_stat_field(stat, 3), Some(1233)); // ppid
        assert_eq!(parse_stat_field(stat, 4), Some(1234)); // pgrp
        assert_eq!(parse_stat_field(stat, 7), Some(1234)); // tpgid
    }
}
