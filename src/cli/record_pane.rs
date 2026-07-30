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
fn pane_hosts_this_process(pane: &str) -> Option<bool> {
    let output = std::process::Command::new("tmux")
        .args(["display-message", "-p", "-t", pane, "#{pane_pid}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let pane_pid: u32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;

    let mut pid = std::process::id();
    for _ in 0..64 {
        if pid == pane_pid {
            return Some(true);
        }
        if pid <= 1 {
            break;
        }
        pid = parent_pid(pid)?;
    }
    Some(false)
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
