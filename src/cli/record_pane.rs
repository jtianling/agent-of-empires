//! Hidden `aoe __record-pane` capture subcommand.
//!
//! The installed agent status hook shells out to this subcommand on hook
//! events. It reads the pane id from the environment (`$AOE_TMUX_PANE` first,
//! then `$TMUX_PANE`) and upserts a `pane_live` row keyed by it. It works for
//! both AoE-launched and hand-launched agents, so it does NOT depend on
//! `$AOE_INSTANCE_ID`.
//!
//! The native session id comes from the source the agent declares in the
//! registry; today every agent's arrives as `session_id` in the hook's stdin
//! JSON. The working directory keeps its own chain (stdin `cwd`, then `$PWD`).
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

fn pane_id_from_env() -> Option<String> {
    [crate::hooks::AOE_PANE_ENV, "TMUX_PANE"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
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
    // Only capture inside tmux: the pane id is the per-pane keystone. Outside
    // tmux there is nothing to key on, so no row is written.
    //
    // The agent-supplied pane wins over `$TMUX_PANE`, because an agent only
    // sets it when its hooks run somewhere `$TMUX_PANE` names a pane other
    // than the agent's own. Falling back the other way would let that stale
    // value claim a pane the agent has nothing to do with.
    let tmux_pane = match pane_id_from_env() {
        Some(p) => p,
        None => return Ok(()),
    };

    // Read stdin whatever the agent is: the hook pipes it, and an unread pipe
    // is the agent's problem. An agent whose id does not come from here still
    // has a `cwd` on it, and one whose stdin is not JSON at all is not a reason
    // to skip a capture whose id came from elsewhere.
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let parsed: HookStdin = serde_json::from_str(&buf).unwrap_or_default();

    let agent = args.agent.clone().unwrap_or_else(|| "claude".to_string());

    // The id comes from the source this agent declares. That is what stops a
    // Codex pane being recorded under the id on its stdin, which is a real
    // value that identifies something other than its conversation.
    //
    // An agent AoE has no hook configuration for keeps the stdin id. Nothing
    // AoE installs invokes this for such an agent -- captures come from hooks,
    // and a hookless agent has none -- so the caller here is stating the id
    // outright rather than having one guessed for it.
    let source = crate::agents::get_agent(&agent)
        .and_then(|a| a.hook_config.as_ref())
        .map(|hooks| hooks.session_id_source)
        .unwrap_or(crate::agents::SessionIdSource::HookStdin);

    let session_id = match source {
        crate::agents::SessionIdSource::HookStdin => parsed.session_id,
        crate::agents::SessionIdSource::EnvVar(name) => std::env::var(name).ok(),
    };
    let session_id = match session_id {
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
