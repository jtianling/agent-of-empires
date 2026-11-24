//! Hidden `aoe __record-pane` capture subcommand.
//!
//! The installed agent status hook shells out to this subcommand on hook
//! events. It reads `$TMUX_PANE` from the environment and upserts a `pane_live`
//! row keyed by the pane id. It works for both AoE-launched and hand-launched
//! agents, so it does NOT depend on `$AOE_INSTANCE_ID`.
//!
//! The native session id and working directory come from the hook's stdin JSON
//! (`session_id`, `cwd`); the working directory falls back to `$PWD`.
//!
//! `$TMUX_PANE` is trusted only after an ancestry check: the pane it names must
//! be the pane this process is actually running in. A hook that executes
//! somewhere else -- Codex's shared `--remote` app-server is the measured case,
//! where every session's hooks inherited the daemon's own `$TMUX_PANE` --
//! would otherwise claim a pane belonging to a different session entirely, and
//! recovery acts on those rows. Verification is positive-only: a pane whose
//! ownership cannot be checked (no tmux server reachable, pane gone) is
//! accepted, a pane that checkably belongs to someone else is not.
//!
//! It MUST never block or fail the agent: any error (no tmux pane, bad JSON,
//! locked db) results in a clean exit 0 with no row written.

use std::io::Read;

use clap::Args;
use serde::Deserialize;

#[derive(Args)]
pub struct RecordPaneArgs {
    /// Agent name (e.g. "claude"). Defaults to "claude" when omitted.
    #[arg(long)]
    agent: Option<String>,
}

#[derive(Deserialize, Default)]
struct HookStdin {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

/// Whether the pane named by `$TMUX_PANE` hosts this process, best-effort.
///
/// `Some(false)` only on a positive mismatch: tmux resolved the pane and its
/// root process is not among this process's ancestors. `None` when the
/// question cannot be answered (tmux unreachable, pane unknown, ps failure),
/// which callers treat as "no evidence against".
///
/// The answer is derived from the server's own pane list rather than by asking
/// it to resolve `$TMUX_PANE`. Resolving the id assumes the server being asked
/// is the one hosting this pane, and nothing establishes that: the socket comes
/// from a profile that may be inherited or derived from the working directory,
/// while pane ids are small integers that repeat across servers. A server that
/// merely happens to have an id by that name answers confidently about someone
/// else's pane, and the capture is dropped.
///
/// Searching this process's ancestry in the pane list inverts that. A server
/// hosting none of this process's ancestors says nothing about it, so it yields
/// `None` rather than a verdict. Only the server that does host this process can
/// produce a mismatch, which is the case the check exists to catch.
///
/// The consequence is a coverage limit worth naming: the socket is always
/// derived from a profile, and with none in the environment that profile comes
/// from the working directory, which practically never names the server hosting
/// this process. Such a call is therefore always unanswerable, so the check is
/// in force only for a capture that carries a profile. It fails open, not shut,
/// but it does not guard a hand-started agent that inherited no profile.
pub(crate) fn pane_hosts_this_process(pane: &str) -> Option<bool> {
    let output = crate::tmux::tmux_command()
        .args(["list-panes", "-a", "-F", "#{pane_id} #{pane_pid}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let panes: Vec<(&str, u32)> = listing
        .lines()
        .filter_map(|line| {
            let (id, pid) = line.split_once(' ')?;
            Some((id, pid.trim().parse().ok()?))
        })
        .collect();

    let mut pid = std::process::id();
    for _ in 0..64 {
        if let Some((id, _)) = panes.iter().find(|(_, pane_pid)| *pane_pid == pid) {
            return Some(*id == pane);
        }
        if pid <= 1 {
            break;
        }
        pid = parent_pid(pid)?;
    }
    None
}

fn parent_pid(pid: u32) -> Option<u32> {
    let output = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Run the capture. Always returns success; capture failures are swallowed so
/// the hook never blocks or errors the agent.
pub fn run(profile: &str, args: RecordPaneArgs) {
    if args.agent.as_deref() == Some("antigravity") {
        if let Err(error) = capture_antigravity(profile) {
            tracing::warn!("Antigravity capture skipped: {error:#}");
            eprintln!("AoE could not record the Antigravity conversation: {error:#}");
        }
        println!("{{}}");
        return;
    }
    if let Err(e) = try_capture(profile, &args) {
        // Best-effort: log at debug only. The hook must still exit 0.
        tracing::debug!("__record-pane capture skipped: {}", e);
    }
}

fn try_capture(profile: &str, args: &RecordPaneArgs) -> anyhow::Result<()> {
    // Only capture inside tmux: $TMUX_PANE is the per-pane keystone. Outside
    // tmux there is nothing to key on, so no row is written.
    let tmux_pane = match std::env::var("TMUX_PANE") {
        Ok(p) if !p.is_empty() => p,
        _ => return Ok(()),
    };
    if pane_hosts_this_process(&tmux_pane) == Some(false) {
        return Ok(());
    }

    // Read stdin whatever the agent is: the hook pipes it, and an unread pipe
    // is the agent's problem.
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let parsed: HookStdin = serde_json::from_str(&buf).unwrap_or_default();

    let agent = args.agent.clone().unwrap_or_else(|| "claude".to_string());

    let session_id = match parsed.session_id {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(()),
    };

    let cwd = parsed
        .cwd
        .filter(|c| !c.is_empty())
        .or_else(|| std::env::var("PWD").ok())
        .unwrap_or_default();

    let store = crate::db::Store::open_with_schema(profile)?;
    store.upsert_pane_live(&tmux_pane, &agent, &session_id, &cwd, crate::db::now_unix())?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AntigravityStart {
    conversation_id: String,
}

fn capture_antigravity(profile: &str) -> anyhow::Result<()> {
    let (Ok(instance), Ok(pane)) = (std::env::var("AOE_INSTANCE_ID"), std::env::var("TMUX_PANE"))
    else {
        return Ok(());
    };
    if !crate::agents::antigravity::is_launch_descendant()? {
        return Ok(());
    }
    anyhow::ensure!(
        !instance.is_empty()
            && pane
                .strip_prefix('%')
                .is_some_and(|s| s.parse::<u32>().is_ok()),
        "Invalid Antigravity hook context"
    );
    let event: AntigravityStart = serde_json::from_reader(std::io::stdin().lock())?;
    anyhow::ensure!(
        crate::hooks::antigravity::valid_id(&event.conversation_id),
        "Invalid Antigravity conversation UUID"
    );
    persist_antigravity_event(
        profile,
        &instance,
        &pane,
        &event.conversation_id,
        std::time::Duration::from_secs(3),
        || pane_hosts_this_process(&pane) == Some(true),
    )
}

fn persist_antigravity_event(
    profile: &str,
    instance: &str,
    pane: &str,
    id: &str,
    timeout: std::time::Duration,
    owns_pane: impl Fn() -> bool,
) -> anyhow::Result<()> {
    let storage = crate::session::Storage::new(profile)?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let reason = match storage.try_operation_lock()? {
            None => "profile operation lock is busy",
            Some(_guard) => match crate::db::Store::open_antigravity_capture(profile)? {
                None => "profile database has not been created",
                Some(store) => {
                    anyhow::ensure!(
                        owns_pane(),
                        "Cannot prove Antigravity hook belongs to pane {pane}"
                    );
                    match store.capture_antigravity_id(instance, pane, id) {
                        Ok(true) => return Ok(()),
                        Ok(false) => "no matching instance and pane binding",
                        Err(error)
                            if matches!(
                                error
                                    .downcast_ref::<rusqlite::Error>()
                                    .and_then(rusqlite::Error::sqlite_error_code),
                                Some(
                                    rusqlite::ErrorCode::DatabaseBusy
                                        | rusqlite::ErrorCode::DatabaseLocked
                                )
                            ) =>
                        {
                            "SQLite database is busy"
                        }
                        Err(error) => return Err(error),
                    }
                }
            },
        };
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        anyhow::ensure!(
            !remaining.is_zero(),
            "Timed out recording Antigravity pane {pane} for instance {instance}: {reason}"
        );
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(100)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    #[serial_test::serial]
    fn antigravity_hook_lock_deadline_and_unproven_owner() {
        let dir = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(dir.path());
        let profile = "capture-test";
        let store = crate::db::Store::open_with_schema(profile).unwrap();
        store
            .record_launched_slot_if_absent("s", 0, "antigravity", "/same", "%1", "", 1)
            .unwrap();
        let id = "ec769e89-8c51-4b1a-9094-00775eb6674d";
        let save = |owns| {
            persist_antigravity_event(profile, "s", "%1", id, Duration::from_millis(20), || owns)
        };
        let storage = crate::session::Storage::new(profile).unwrap();
        let held = storage.operation_lock().unwrap();
        std::thread::scope(|scope| {
            let join = scope.spawn(|| save(true));
            let start = Instant::now();
            while !join.is_finished() && start.elapsed() < Duration::from_secs(1) {
                std::thread::sleep(Duration::from_millis(5));
            }
            drop(held);
            assert!(join
                .join()
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("operation lock is busy"));
        });
        let db = rusqlite::Connection::open(crate::db::db_path(profile).unwrap()).unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert!(save(true)
            .unwrap_err()
            .to_string()
            .contains("SQLite database is busy"));
        db.execute_batch("ROLLBACK").unwrap();
        assert!(save(false).is_err());
        assert!(store.read_slots_for_instance("s").unwrap()[0]
            .native_session_id
            .is_empty());
        save(true).unwrap();
        assert_eq!(
            store.read_slots_for_instance("s").unwrap()[0].native_session_id,
            id
        );
    }
}
