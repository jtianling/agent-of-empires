//! Bind an AoE-launched Codex pane to its conversation via Codex's rollout files.
//!
//! Codex cannot be tracked through hooks: its `--remote` clients execute hooks
//! inside a shared app-server whose environment was frozen at daemon start, so
//! a hook there sees another pane's `$TMUX_PANE` and would claim a pane that
//! belongs to a different session. What Codex does reliably produce, in every
//! launch mode, is one rollout file per conversation:
//!
//! ```text
//! ~/.codex/sessions/YYYY/MM/DD/rollout-YYYY-MM-DDTHH-MM-SS-<thread-uuid>.jsonl
//! ```
//!
//! whose first line is a `session_meta` record carrying the conversation's
//! `cwd`. AoE knows the rest at launch time: the pane, the instance, when the
//! pane's process started, and the project path. The reconciler calls in here
//! for a Codex instance whose primary pane has no capture yet, and the claim is
//! the earliest rollout file created after the pane's process started, in the
//! instance's working directory, that no other pane or slot already owns.
//!
//! A resumed pane (`codex resume <token>`) keeps its original rollout file,
//! whose timestamp predates the respawn, so it never re-matches here -- its
//! durable slot already carries the right conversation. A forked pane creates
//! a fresh rollout and is claimed like a fresh launch.

use std::collections::HashSet;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone};

use crate::db::Store;
use crate::session::Instance;

/// Allowance for the pane's shell starting before Codex stamps the rollout
/// name, and for coarse clocks: a rollout this much older than the pane's
/// process still counts as created by it.
const LAUNCH_SLACK_SECS: i64 = 2;

/// One matched rollout file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloutMatch {
    pub thread_id: String,
    pub cwd: String,
}

/// Claim a rollout for one pane of a managed session, if one is due.
///
/// No-op unless the pane has no `pane_live` capture yet. Failures are
/// swallowed: the next reconcile tick tries again.
///
/// The instance-level conditions -- that its tool is Codex, and that its
/// command has not been overridden (an override is the user's own program) --
/// describe the instance's own agent pane, so they gate `is_primary` only. A
/// pane beside it may run a different agent than the instance's tool, and the
/// override names what AoE launches for the instance, not what is running
/// there. Every pane is still judged by the positive evidence below: a process
/// in its tree invoking Codex.
pub fn maybe_claim_for_pane(store: &Store, inst: &Instance, pane_id: &str, is_primary: bool) {
    if !instance_permits_claim(inst, is_primary) {
        return;
    }
    match store.read_pane_live(pane_id) {
        Ok(None) => {}
        _ => return,
    }
    let Some(pane_pid) = pane_root_pid(pane_id) else {
        return;
    };
    // A conversation is only claimed for a pane that is actually running
    // Codex. Without this, a pane whose Codex has exited (or a shell pane
    // that merely belongs to a codex-tool instance) would be bound to
    // whatever conversation happened to start in the same directory.
    if !process_tree_runs_codex(pane_pid) {
        return;
    }
    let Some(launched_at) = process_start_unix(pane_pid) else {
        return;
    };
    let claimed = match store.claimed_native_session_ids() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("codex rollout: cannot list claimed ids: {}", e);
            return;
        }
    };
    let root = codex_sessions_root();
    let Some(found) = find_rollout(&root, &inst.project_path, launched_at, &claimed) else {
        return;
    };
    match store.upsert_pane_live(
        pane_id,
        "codex",
        &found.thread_id,
        &found.cwd,
        crate::db::now_unix(),
    ) {
        Ok(()) => tracing::info!(
            "codex rollout: pane {} bound to thread {}",
            pane_id,
            found.thread_id
        ),
        Err(e) => tracing::debug!("codex rollout: claim for {} failed: {}", pane_id, e),
    }
}

/// Whether the instance-level conditions permit claiming for this pane.
///
/// They describe the instance's own agent pane, so they answer for the primary
/// pane alone. A pane beside it is judged by what is running in it.
fn instance_permits_claim(inst: &Instance, is_primary: bool) -> bool {
    !is_primary || (inst.tool == "codex" && !inst.has_command_override())
}

/// `$CODEX_HOME/sessions`, defaulting the home part to `~/.codex`.
fn codex_sessions_root() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
        .unwrap_or_default()
        .join("sessions")
}

/// Root process id of a tmux pane.
fn pane_root_pid(pane_id: &str) -> Option<u32> {
    let output = crate::tmux::tmux_command()
        .args(["display-message", "-p", "-t", pane_id, "#{pane_pid}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Whether a process or any of its descendants is invoking Codex.
///
/// Matched on the command line rather than the process name: Codex installed
/// through npm runs behind a `node` shim, so the kernel reports `node`, but
/// the argv still names the `codex` entry point.
fn process_tree_runs_codex(root_pid: u32) -> bool {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-Ao", "pid=,ppid=,command="])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    let mut commands: std::collections::HashMap<u32, &str> = std::collections::HashMap::new();
    for line in listing.lines() {
        let mut parts = line.split_whitespace();
        let (Some(pid), Some(ppid)) = (
            parts.next().and_then(|p| p.parse::<u32>().ok()),
            parts.next().and_then(|p| p.parse::<u32>().ok()),
        ) else {
            continue;
        };
        children.entry(ppid).or_default().push(pid);
        commands.insert(pid, line);
    }
    let mut queue = vec![root_pid];
    while let Some(pid) = queue.pop() {
        if commands.get(&pid).is_some_and(|cmd| cmd.contains("codex")) {
            return true;
        }
        if let Some(kids) = children.get(&pid) {
            queue.extend(kids);
        }
    }
    false
}

/// Unix start time of a process, from `ps` elapsed time.
fn process_start_unix(pid: u32) -> Option<i64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "etime=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let elapsed = parse_etime(String::from_utf8_lossy(&output.stdout).trim())?;
    Some(crate::db::now_unix() - elapsed)
}

/// Parse `ps` etime (`[[dd-]hh:]mm:ss`) into seconds.
fn parse_etime(s: &str) -> Option<i64> {
    let (days, rest) = match s.split_once('-') {
        Some((d, rest)) => (d.parse::<i64>().ok()?, rest),
        None => (0, s),
    };
    let parts: Vec<i64> = rest
        .split(':')
        .map(|p| p.parse::<i64>())
        .collect::<Result<_, _>>()
        .ok()?;
    let (h, m, sec) = match parts.as_slice() {
        [m, s] => (0, *m, *s),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    Some(((days * 24 + h) * 60 + m) * 60 + sec)
}

/// Find the earliest unclaimed rollout created at or after `launched_at` whose
/// conversation ran in `project_path`.
pub fn find_rollout(
    sessions_root: &Path,
    project_path: &str,
    launched_at: i64,
    claimed: &HashSet<String>,
) -> Option<RolloutMatch> {
    let cutoff = launched_at - LAUNCH_SLACK_SECS;
    let earliest_day = Local
        .timestamp_opt(cutoff, 0)
        .single()
        .map(|t| t.date_naive())?;

    let mut candidates: Vec<(i64, String, PathBuf)> = Vec::new();
    for (date, day_dir) in day_dirs(sessions_root) {
        if date < earliest_day {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&day_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some((ts, thread_id)) = parse_rollout_name(&name.to_string_lossy()) else {
                continue;
            };
            if ts < cutoff || claimed.contains(&thread_id) {
                continue;
            }
            candidates.push((ts, thread_id, entry.path()));
        }
    }
    candidates.sort();

    for (_, thread_id, path) in candidates {
        let Some(cwd) = rollout_cwd(&path) else {
            continue;
        };
        if same_dir(&cwd, project_path) {
            return Some(RolloutMatch { thread_id, cwd });
        }
    }
    None
}

/// Directory equality that survives symlinked prefixes (macOS `/var` vs
/// `/private/var`) and trailing slashes.
fn same_dir(a: &str, b: &str) -> bool {
    if a.trim_end_matches('/') == b.trim_end_matches('/') {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Enumerate `root/YYYY/MM/DD` leaf directories with their dates.
fn day_dirs(root: &Path) -> Vec<(NaiveDate, PathBuf)> {
    fn numeric_subdirs(dir: &Path) -> Vec<(u32, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().parse::<u32>().ok()?;
                e.path().is_dir().then(|| (n, e.path()))
            })
            .collect()
    }
    let mut out = Vec::new();
    for (year, year_dir) in numeric_subdirs(root) {
        for (month, month_dir) in numeric_subdirs(&year_dir) {
            for (day, day_dir) in numeric_subdirs(&month_dir) {
                if let Some(date) = NaiveDate::from_ymd_opt(year as i32, month, day) {
                    out.push((date, day_dir));
                }
            }
        }
    }
    out
}

/// Parse `rollout-YYYY-MM-DDTHH-MM-SS-<uuid>.jsonl` into (local unix ts, uuid).
fn parse_rollout_name(name: &str) -> Option<(i64, String)> {
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    // The timestamp is a fixed-width prefix; the remainder is the thread uuid.
    let ts_len = "2026-07-30T12-12-05".len();
    if !stem.is_char_boundary(ts_len) || stem.len() <= ts_len {
        return None;
    }
    let (ts_part, thread_id) = stem.split_at(ts_len);
    let thread_id = thread_id.strip_prefix('-')?;
    if thread_id.is_empty() {
        return None;
    }
    let naive = NaiveDateTime::parse_from_str(ts_part, "%Y-%m-%dT%H-%M-%S").ok()?;
    let ts = Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.timestamp())?;
    Some((ts, thread_id.to_string()))
}

/// Read the conversation `cwd` from a rollout's first line (`session_meta`).
fn rollout_cwd(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    // The first line carries the full base instructions, so it can be large,
    // but it is one line: cap what a broken file can make us read.
    let mut reader = std::io::BufReader::new(file).take(4 * 1024 * 1024);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let value: serde_json::Value = serde_json::from_str(&line).ok()?;
    value["payload"]["cwd"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn write_rollout(root: &Path, ts: i64, uuid: &str, cwd: &str) {
        let local = Local.timestamp_opt(ts, 0).single().unwrap();
        let day_dir = root
            .join(format!("{:04}", local.year()))
            .join(format!("{:02}", local.month()))
            .join(format!("{:02}", local.day()));
        std::fs::create_dir_all(&day_dir).unwrap();
        let name = format!("rollout-{}-{uuid}.jsonl", local.format("%Y-%m-%dT%H-%M-%S"));
        let meta = serde_json::json!({
            "timestamp": "x",
            "type": "session_meta",
            "payload": { "session_id": uuid, "cwd": cwd }
        });
        std::fs::write(day_dir.join(name), format!("{meta}\n")).unwrap();
    }

    #[test]
    fn claims_the_earliest_rollout_after_launch_in_the_project() {
        let tmp = tempfile::tempdir().unwrap();
        let launch = crate::db::now_unix() - 3600;
        write_rollout(tmp.path(), launch - 300, "old-before-launch", "/proj");
        write_rollout(tmp.path(), launch + 5, "mine", "/proj");
        write_rollout(tmp.path(), launch + 60, "later-neighbor", "/proj");
        write_rollout(tmp.path(), launch + 3, "other-project", "/elsewhere");

        let found = find_rollout(tmp.path(), "/proj", launch, &HashSet::new()).unwrap();
        assert_eq!(found.thread_id, "mine");
        assert_eq!(found.cwd, "/proj");
    }

    #[test]
    fn a_claimed_thread_is_never_reassigned() {
        let tmp = tempfile::tempdir().unwrap();
        let launch = crate::db::now_unix() - 3600;
        write_rollout(tmp.path(), launch + 5, "already-owned", "/proj");
        write_rollout(tmp.path(), launch + 60, "free", "/proj");

        let claimed: HashSet<String> = ["already-owned".to_string()].into();
        let found = find_rollout(tmp.path(), "/proj", launch, &claimed).unwrap();
        assert_eq!(found.thread_id, "free");
    }

    #[test]
    fn a_resumed_conversation_predating_the_launch_does_not_match() {
        let tmp = tempfile::tempdir().unwrap();
        let launch = crate::db::now_unix() - 60;
        write_rollout(tmp.path(), launch - 3600, "resumed-original", "/proj");

        assert_eq!(
            find_rollout(tmp.path(), "/proj", launch, &HashSet::new()),
            None
        );
    }

    #[test]
    fn trailing_slashes_do_not_defeat_the_cwd_match() {
        let tmp = tempfile::tempdir().unwrap();
        let launch = crate::db::now_unix() - 3600;
        write_rollout(tmp.path(), launch + 5, "mine", "/proj/");

        let found = find_rollout(tmp.path(), "/proj", launch, &HashSet::new()).unwrap();
        assert_eq!(found.thread_id, "mine");
    }

    #[test]
    fn two_panes_of_one_session_take_different_conversations() {
        let tmp = tempfile::tempdir().unwrap();
        let first_launch = crate::db::now_unix() - 3600;
        let second_launch = first_launch + 120;
        write_rollout(tmp.path(), first_launch + 5, "primary-thread", "/proj");
        write_rollout(tmp.path(), second_launch + 5, "extra-thread", "/proj");

        // The reconciler offers panes in creation order, accumulating claims.
        let mut claimed: HashSet<String> = HashSet::new();
        let first = find_rollout(tmp.path(), "/proj", first_launch, &claimed).unwrap();
        claimed.insert(first.thread_id.clone());
        let second = find_rollout(tmp.path(), "/proj", second_launch, &claimed).unwrap();

        assert_eq!(first.thread_id, "primary-thread");
        assert_eq!(second.thread_id, "extra-thread");
    }

    #[test]
    fn a_later_pane_does_not_take_the_earlier_panes_conversation() {
        let tmp = tempfile::tempdir().unwrap();
        let first_launch = crate::db::now_unix() - 3600;
        let second_launch = first_launch + 120;
        write_rollout(tmp.path(), first_launch + 5, "primary-thread", "/proj");

        let mut claimed: HashSet<String> = HashSet::new();
        claimed.insert(
            find_rollout(tmp.path(), "/proj", first_launch, &claimed)
                .unwrap()
                .thread_id,
        );

        assert_eq!(
            find_rollout(tmp.path(), "/proj", second_launch, &claimed),
            None
        );
    }

    #[test]
    fn instance_conditions_answer_for_the_primary_pane_only() {
        let mut inst = Instance::new("test", "/proj");
        inst.tool = "claude".to_string();
        assert!(!instance_permits_claim(&inst, true));
        assert!(instance_permits_claim(&inst, false));

        inst.tool = "codex".to_string();
        inst.command = "codex --instance-override".to_string();
        assert!(!instance_permits_claim(&inst, true));
        assert!(instance_permits_claim(&inst, false));

        inst.command = String::new();
        assert!(instance_permits_claim(&inst, true));
    }

    #[test]
    fn parses_ps_etime_forms() {
        assert_eq!(parse_etime("05:09"), Some(309));
        assert_eq!(parse_etime("01:02:03"), Some(3723));
        assert_eq!(parse_etime("2-01:02:03"), Some(2 * 86400 + 3723));
        assert_eq!(parse_etime("bogus"), None);
    }

    #[test]
    fn rollout_names_that_are_not_rollouts_are_ignored() {
        assert_eq!(
            parse_rollout_name("rollout-2026-07-30T12-12-05-abc.jsonl").map(|(_, id)| id),
            Some("abc".to_string())
        );
        assert_eq!(
            parse_rollout_name("rollout-2026-07-30T12-12-05-.jsonl"),
            None
        );
        assert_eq!(parse_rollout_name("notes.txt"), None);
        assert_eq!(parse_rollout_name("rollout-garbage.jsonl"), None);
    }
}
