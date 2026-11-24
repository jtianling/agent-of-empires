mod client;
mod complete;
mod legacy;
mod pending;
mod resolve;
mod stage;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use complete::{commit_opencode, wait_committed};
pub use complete::{run, CompleteArgs};
pub(crate) use stage::{
    begin_launch, bootstrap_command, ensure_for_slot, prepare_switch, runtime_generation,
};

pub(crate) fn has_prepared(profile: &str, id: &str, slot: i64) -> anyhow::Result<bool> {
    Ok(pending::load(profile, id, slot)?
        .and_then(|pending| pending.response)
        .is_some_and(|response| response.state == types::State::Prepared))
}

pub(crate) fn handles_bootstrap(profile: &str, id: &str, pane: &str) -> anyhow::Result<bool> {
    let slot = crate::db::Store::open_with_schema(profile)?
        .read_slots_for_instance(id)?
        .into_iter()
        .find(|slot| slot.tmux_pane == pane);
    slot.map(|slot| has_prepared(profile, id, slot.slot))
        .transpose()
        .map(|value| value.unwrap_or(false))
}

use anyhow::{ensure, Context, Result};
use pending::Pending;
use serde_json::json;
use types::{Response, Runtime, State};

fn control(pending: &Pending) -> Result<serde_json::Value> {
    let response = pending
        .response
        .as_ref()
        .context("Handoff has not been prepared")?;
    Ok(
        json!({"protocol_version": 1, "identity_key": pending.identity_key,
        "fence": response.fence}),
    )
}

fn refresh(profile: &str, pending: &mut Pending) -> Result<Response> {
    let response = client::call("status", control(pending)?, &pending.identity_key)?;
    pending.validate_response(&response)?;
    pending.controlled_identity |= response.state == State::Committed;
    pending.response = Some(response.clone());
    pending::save(profile, pending)?;
    ensure!(
        matches!(response.state, State::Prepared | State::Committed),
        "Runtime handoff is {:?}; resolve the previous handoff before retrying",
        response.state
    );
    capture_native(profile, pending, &response)?;
    Ok(response)
}

fn commit(profile: &str, pending: &mut Pending, runtime: Runtime) -> Result<Response> {
    runtime.validate()?;
    ensure!(
        runtime.kind() == types::agent_type(&pending.target_tool)?,
        "Runtime handoff target type changed"
    );
    pending.target_runtime = Some(runtime.clone());
    pending::save(profile, pending)?;
    let mut request = control(pending)?;
    request["runtime"] = serde_json::to_value(&runtime)?;
    let response = client::call("commit", request, &pending.identity_key)?;
    pending.validate_response(&response)?;
    ensure!(
        response.state == State::Committed,
        "Runtime handoff has not committed"
    );
    let current = response
        .current
        .as_ref()
        .context("Committed handoff has no identity")?;
    ensure!(
        current.agent_type.as_deref() == Some(runtime.kind()),
        "Committed handoff has another agent type"
    );
    let actual = current
        .runtime
        .as_ref()
        .context("Committed handoff has no runtime")?;
    ensure!(
        same_runtime(actual, &runtime),
        "Committed handoff has another runtime"
    );
    pending.controlled_identity = true;
    pending.response = Some(response.clone());
    pending::save(profile, pending)?;
    capture_native(profile, pending, &response)?;
    Ok(response)
}

fn capture_native(profile: &str, pending: &Pending, response: &Response) -> Result<()> {
    if response.state != State::Committed {
        return Ok(());
    }
    let current = response
        .current
        .as_ref()
        .context("Committed handoff identity is missing")?;
    let native = match current.runtime.as_ref() {
        Some(Runtime::Codex { thread_id, .. }) => thread_id,
        Some(Runtime::OpenCode { session_id, .. }) => session_id,
        _ => return Ok(()),
    };
    crate::db::Store::open_with_schema(profile)?.record_handoff_native(
        &pending.instance_id,
        pending.slot,
        &pending.identity_key,
        &pending.target_tool,
        native,
    )
}

fn same_runtime(actual: &Runtime, requested: &Runtime) -> bool {
    match (actual, requested) {
        (
            Runtime::Claude {
                ui_pid: left,
                channel_session_id: channel,
            },
            Runtime::Claude {
                ui_pid: right,
                channel_session_id: wanted,
            },
        ) => {
            left == right
                && wanted
                    .as_ref()
                    .map_or(true, |wanted| channel.as_ref() == Some(wanted))
        }
        (
            Runtime::Codex {
                thread_id: left,
                ws_url: url,
            },
            Runtime::Codex {
                thread_id: right,
                ws_url: wanted,
            },
        ) => left == right && url::Url::parse(url).ok() == url::Url::parse(wanted).ok(),
        (
            Runtime::OpenCode {
                session_id: left,
                base_url: url,
            },
            Runtime::OpenCode {
                session_id: right,
                base_url: wanted,
            },
        ) => left == right && url::Url::parse(url).ok() == url::Url::parse(wanted).ok(),
        _ => false,
    }
}
