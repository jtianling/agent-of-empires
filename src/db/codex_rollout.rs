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
//! `cwd` and `originator`. AoE knows the rest at launch time: the pane, the
//! instance, when the pane's process started, and the project path. The
//! reconciler calls in here for each pane of a managed session, and the claim is
//! the earliest rollout file created after the pane's process started, in the
//! instance's working directory, written by an interactive Codex, that no other
//! pane or slot already owns.
//!
//! A pane is claimed for when it has no capture, and again when its capture
//! predates the process now in it -- restarting a live session respawns the pane
//! and keeps its pane id, so a capture can outlive the conversation it names.
//!
//! A resumed pane (`codex resume <token>`) is the exception: its capture also
//! predates the respawn, but the conversation is deliberately unchanged, and its
//! command line carries that conversation's id. That says so directly and
//! outranks the timestamps. A forked pane creates a fresh rollout and is claimed
//! like a fresh launch.

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

/// The fields of a rollout's `session_meta` line that the match reads.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RolloutHeader {
    cwd: String,
    /// Absent in rollouts that do not record one, which stay eligible.
    originator: Option<String>,
}

/// The originator Codex records for a conversation started by its TUI, which is
/// what a pane runs. Observed with a `source` of both `vscode` and `cli`, so
/// `source` is not the discriminator.
const CODEX_INTERACTIVE_ORIGINATOR: &str = "codex-tui";

/// Originators that name a Codex entry point which never runs in a pane. A
/// `codex exec` invoked by a script in the same repository writes a rollout with
/// the same `cwd` as the panes there, and would otherwise be an eligible match.
///
/// Deliberately a list of what to reject rather than what to accept. Rejecting
/// an unrecognized originator would stop every pane from being adopted if a
/// future Codex renamed the interactive one, and an unadopted pane has no slot,
/// so it is silently dropped from restart and recovery -- the failure this
/// filter exists to prevent. Missing one newly added non-interactive originator
/// only leaves the previous behavior in place for it.
const CODEX_NON_INTERACTIVE_ORIGINATORS: &[&str] = &["codex_exec"];

/// Claim a rollout for one pane of a managed session, if one is due.
///
/// A claim is due when the pane has no capture, and also when its capture was
/// recorded before the process now in the pane started -- that capture names a
/// conversation belonging to a process that no longer exists. Restarting a live
/// session respawns the pane in place and keeps its pane id, so without this the
/// pane would keep describing what it ran before the restart forever. Failures
/// are swallowed: the next reconcile tick tries again.
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
    let existing = match store.read_pane_live(pane_id) {
        Ok(existing) => existing,
        Err(e) => {
            tracing::debug!("codex rollout: cannot read capture for {}: {}", pane_id, e);
            return;
        }
    };
    if let Some(existing) = &existing {
        let superseded = capture_is_superseded(existing.updated_at, launched_at, || {
            process_tree_mentions(pane_pid, &existing.native_session_id)
        });
        if !superseded {
            return;
        }
    }
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
        Ok(()) => match existing {
            Some(stale) => tracing::info!(
                "codex rollout: pane {} rebound to thread {} (was {}, from before this process)",
                pane_id,
                found.thread_id,
                stale.native_session_id
            ),
            None => tracing::info!(
                "codex rollout: pane {} bound to thread {}",
                pane_id,
                found.thread_id
            ),
        },
        Err(e) => tracing::debug!("codex rollout: claim for {} failed: {}", pane_id, e),
    }
}

/// Whether a pane's existing capture has been left behind by the process now in
/// that pane, so the pane should be claimed for again.
///
/// Two conditions, in order of cost. The capture must predate the process: a
/// near-tie resolves to "not superseded", because `process_start_unix` derives
/// from `ps` elapsed seconds against the current clock and is good to about a
/// second, and the two errors are not symmetric -- calling a live capture
/// superseded sends a correctly bound pane looking for another conversation,
/// while calling a superseded one live only defers the correction to the next
/// reconcile tick. In practice the two times are tens of seconds apart either
/// way, so the margin is a guard rather than a working part.
///
/// And the pane must not still be running that same conversation.
/// `pane_runs_that_conversation` is deferred because it costs a process listing
/// and only matters once the timestamps say otherwise. It is what keeps a
/// resumed pane bound: `R` respawns it, so its capture necessarily predates the
/// new process, but `codex resume <token>` carries the conversation's own id on
/// the command line, and that is direct evidence where the timestamps are only
/// circumstantial.
fn capture_is_superseded(
    captured_at: i64,
    launched_at: i64,
    pane_runs_that_conversation: impl FnOnce() -> bool,
) -> bool {
    captured_at + LAUNCH_SLACK_SECS < launched_at && !pane_runs_that_conversation()
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
    process_tree_any(root_pid, |cmd| cmd.contains("codex"))
}

/// Whether any process in the pane's tree names `needle` on its command line.
///
/// Used to recognize a resumed pane: `codex resume <thread>` carries the
/// conversation's own id, so a pane still running the conversation its capture
/// records is not superseded, however old that capture is.
fn process_tree_mentions(root_pid: u32, needle: &str) -> bool {
    !needle.is_empty() && process_tree_any(root_pid, |cmd| cmd.contains(needle))
}

/// Whether any process in `root_pid`'s tree has a command line satisfying
/// `matches`.
fn process_tree_any(root_pid: u32, matches: impl Fn(&str) -> bool) -> bool {
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
        if commands.get(&pid).is_some_and(|cmd| matches(cmd)) {
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
        let Some(header) = rollout_header(&path) else {
            continue;
        };
        if !same_dir(&header.cwd, project_path) {
            continue;
        }
        if !originator_can_run_in_a_pane(header.originator.as_deref(), &path) {
            continue;
        }
        return Some(RolloutMatch {
            thread_id,
            cwd: header.cwd,
        });
    }
    None
}

/// Whether a conversation with this originator could have been running in a
/// pane. An absent or unrecognized originator is eligible; see
/// [`CODEX_NON_INTERACTIVE_ORIGINATORS`] for why the filter rejects rather than
/// accepts.
fn originator_can_run_in_a_pane(originator: Option<&str>, path: &Path) -> bool {
    let Some(originator) = originator else {
        return true;
    };
    if CODEX_NON_INTERACTIVE_ORIGINATORS.contains(&originator) {
        tracing::debug!(
            "codex rollout: skipping {} from non-interactive originator '{}'",
            path.display(),
            originator
        );
        return false;
    }
    if originator != CODEX_INTERACTIVE_ORIGINATOR {
        // Not a behavior change, a signal: Codex's originator values have moved
        // and this filter's assumption needs revisiting.
        tracing::warn!(
            "codex rollout: unrecognized originator '{}' in {}; treating it as eligible",
            originator,
            path.display()
        );
    }
    true
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
fn rollout_header(path: &Path) -> Option<RolloutHeader> {
    let file = std::fs::File::open(path).ok()?;
    // The first line carries the full base instructions, so it can be large,
    // but it is one line: cap what a broken file can make us read.
    let mut reader = std::io::BufReader::new(file).take(4 * 1024 * 1024);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let value: serde_json::Value = serde_json::from_str(&line).ok()?;
    Some(RolloutHeader {
        cwd: value["payload"]["cwd"].as_str()?.to_string(),
        originator: value["payload"]["originator"].as_str().map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn write_rollout(root: &Path, ts: i64, uuid: &str, cwd: &str) {
        write_rollout_from(root, ts, uuid, cwd, Some(CODEX_INTERACTIVE_ORIGINATOR));
    }

    fn write_rollout_from(root: &Path, ts: i64, uuid: &str, cwd: &str, originator: Option<&str>) {
        let local = Local.timestamp_opt(ts, 0).single().unwrap();
        let day_dir = root
            .join(format!("{:04}", local.year()))
            .join(format!("{:02}", local.month()))
            .join(format!("{:02}", local.day()));
        std::fs::create_dir_all(&day_dir).unwrap();
        let name = format!("rollout-{}-{uuid}.jsonl", local.format("%Y-%m-%dT%H-%M-%S"));
        let mut payload = serde_json::json!({ "session_id": uuid, "cwd": cwd });
        if let Some(originator) = originator {
            payload["originator"] = serde_json::json!(originator);
        }
        let meta = serde_json::json!({
            "timestamp": "x",
            "type": "session_meta",
            "payload": payload
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

    /// A `codex exec` run by a script in the same repository writes a rollout
    /// with the same cwd. Being the earliest match is exactly how it would win.
    #[test]
    fn a_scripted_codex_run_is_skipped_even_when_it_is_the_earliest_match() {
        let tmp = tempfile::tempdir().unwrap();
        let launch = crate::db::now_unix() - 3600;
        write_rollout_from(
            tmp.path(),
            launch + 5,
            "from-a-script",
            "/proj",
            Some("codex_exec"),
        );
        write_rollout(tmp.path(), launch + 60, "from-a-pane", "/proj");

        let found = find_rollout(tmp.path(), "/proj", launch, &HashSet::new()).unwrap();
        assert_eq!(found.thread_id, "from-a-pane");
    }

    #[test]
    fn a_scripted_run_alone_leaves_the_pane_unbound() {
        let tmp = tempfile::tempdir().unwrap();
        let launch = crate::db::now_unix() - 3600;
        write_rollout_from(
            tmp.path(),
            launch + 5,
            "from-a-script",
            "/proj",
            Some("codex_exec"),
        );

        assert_eq!(
            find_rollout(tmp.path(), "/proj", launch, &HashSet::new()),
            None,
            "no binding is better than a binding to a conversation no pane ran"
        );
    }

    /// Rejecting these would stop every pane from being adopted the day Codex
    /// renames its interactive originator. See the constant's documentation.
    #[test]
    fn an_absent_or_unrecognized_originator_stays_eligible() {
        for originator in [None, Some("codex-something-new")] {
            let tmp = tempfile::tempdir().unwrap();
            let launch = crate::db::now_unix() - 3600;
            write_rollout_from(tmp.path(), launch + 5, "thread", "/proj", originator);

            let found = find_rollout(tmp.path(), "/proj", launch, &HashSet::new());
            assert_eq!(
                found.map(|f| f.thread_id),
                Some("thread".to_string()),
                "originator {originator:?} must stay eligible"
            );
        }
    }

    #[test]
    fn the_rollout_header_carries_both_fields_from_one_line() {
        let tmp = tempfile::tempdir().unwrap();
        let ts = crate::db::now_unix();
        write_rollout_from(tmp.path(), ts, "thread", "/proj", Some("codex_exec"));

        let path = std::fs::read_dir(tmp.path())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .and_then(|year| walk_to_file(&year))
            .expect("rollout written");

        let header = rollout_header(&path).unwrap();
        assert_eq!(header.cwd, "/proj");
        assert_eq!(header.originator.as_deref(), Some("codex_exec"));
    }

    fn walk_to_file(dir: &Path) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_file() {
                return Some(path);
            }
            if let Some(found) = walk_to_file(&path) {
                return Some(found);
            }
        }
        None
    }

    /// The claim gate: a capture from before the current process cannot name
    /// what that process is running, so the pane is claimed for again.
    #[test]
    fn a_capture_older_than_the_pane_process_is_superseded() {
        let launched = 1_000_000;

        assert!(
            capture_is_superseded(launched - 600, launched, || false),
            "a capture from before the respawn is superseded"
        );
        assert!(
            !capture_is_superseded(launched + 44, launched, || false),
            "a live pane's capture is written after its process started"
        );
    }

    /// `R` respawns the pane, so a resumed pane's capture necessarily predates
    /// its process -- while naming exactly the conversation it is running.
    #[test]
    fn a_resumed_pane_keeps_its_capture_however_old_it_is() {
        let launched = 1_000_000;

        assert!(
            !capture_is_superseded(launched - 86_400, launched, || true),
            "running that conversation outranks the timestamps"
        );
    }

    /// Calling a live capture superseded sends a correctly bound pane looking
    /// for another conversation; the reverse only defers the fix one tick.
    #[test]
    fn a_near_tie_resolves_to_not_superseded() {
        let launched = 1_000_000;
        let never_run = || panic!("a near-tie must be decided by the timestamps alone");

        assert!(!capture_is_superseded(launched, launched, never_run));
        assert!(!capture_is_superseded(launched - 1, launched, never_run));
        assert!(!capture_is_superseded(
            launched - LAUNCH_SLACK_SECS,
            launched,
            never_run
        ));
        assert!(capture_is_superseded(
            launched - LAUNCH_SLACK_SECS - 1,
            launched,
            || false
        ));
    }

    #[test]
    fn an_empty_conversation_id_never_matches_a_process_tree() {
        assert!(
            !process_tree_mentions(std::process::id(), ""),
            "an empty needle would otherwise match every command line"
        );
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
