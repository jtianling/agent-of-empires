//! Hidden `aoe __record-pane` capture subcommand.
//!
//! The installed agent status hook shells out to this subcommand on hook
//! events. It reads the hook's stdin JSON (`{"session_id": ..., "cwd": ...}`),
//! reads `$TMUX_PANE` from the environment, and upserts a `pane_live` row keyed
//! by the pane id. It works for both AoE-launched and hand-launched agents, so
//! it does NOT depend on `$AOE_INSTANCE_ID`.
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

#[derive(Deserialize)]
struct HookStdin {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
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

    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let parsed: HookStdin = serde_json::from_str(&buf)?;

    let session_id = match parsed.session_id {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(()),
    };
    let cwd = parsed
        .cwd
        .filter(|c| !c.is_empty())
        .or_else(|| std::env::var("PWD").ok())
        .unwrap_or_default();

    let agent = args.agent.clone().unwrap_or_else(|| "claude".to_string());

    let store = crate::db::Store::open_with_schema(profile)?;
    store.upsert_pane_live(&tmux_pane, &agent, &session_id, &cwd, crate::db::now_unix())?;
    Ok(())
}
