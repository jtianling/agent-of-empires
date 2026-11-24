use anyhow::{ensure, Context, Result};
use serde_json::json;

use super::{
    client,
    pending::{self, Pending},
    types::{agent_type, State},
};
use crate::db::AgentSlot;
use crate::session::Instance;

pub(crate) fn prepare_switch(
    profile: &str,
    instance: &Instance,
    slot: &AgentSlot,
    target: &str,
    live_source: bool,
) -> Result<()> {
    if !slot.cross_agent_team
        || (slot.slot == 0 && instance.managed_xats.is_some())
        || instance
            .managed_panes
            .iter()
            .any(|pane| pane.slot == Some(slot.slot) && pane.xats.is_some())
    {
        return Ok(());
    }
    let holder = crate::xats_identity::lookup_holders(vec![slot.xats_identity_key.clone()])
        .into_iter()
        .next()
        .context("xats did not return an identity")?;
    let (name, team, source) = match holder {
        crate::xats_identity::KeyHolder::Held {
            name,
            team,
            agent_type,
            ..
        } => (
            name,
            team,
            agent_type.context("xats identity has no source type")?,
        ),
        crate::xats_identity::KeyHolder::NotFound => return Ok(()),
        holder => anyhow::bail!("Cannot prepare xats identity handoff: {holder:?}"),
    };
    let previous = pending::load(profile, &instance.id, slot.slot)?;
    let controlled = previous.as_ref().is_some_and(|pending| {
        pending.controlled_identity
            || pending
                .response
                .as_ref()
                .is_some_and(|r| r.state == State::Committed)
    });
    if let Some(mut pending) = previous {
        let state = pending.response.as_ref().map(|response| response.state);
        if !pending.prepare_rejected
            && !matches!(
                state,
                Some(State::Committed | State::Aborted | State::Stale)
            )
        {
            ensure!(
                pending.target_tool == target && pending.identity_key == slot.xats_identity_key,
                "Another runtime handoff is pending for this pane"
            );
            return prepare(profile, &mut pending);
        }
        if state == Some(State::Stale) {
            let response = client::call("abort", super::control(&pending)?, &pending.identity_key)?;
            pending.validate_response(&response)?;
            pending.response = Some(response);
            pending::save(profile, &pending)?;
        }
    }
    if source == agent_type(target)? && !controlled {
        return Ok(());
    }
    let mut source_config = slot.pane_config();
    source_config.tool = match source.as_str() {
        "claude-code" => "claude".into(),
        "codex" | "opencode" => source.clone(),
        _ => anyhow::bail!("Unsupported xats source type: {source}"),
    };
    let source_launch = if live_source && source == "codex" && !slot.tmux_pane.is_empty() {
        crate::db::codex_capture::current_launch_id(&slot.tmux_pane)?
    } else {
        None
    };
    let mut pending = Pending {
        request_id: uuid::Uuid::new_v4().to_string(),
        instance_id: instance.id.clone(),
        slot: slot.slot,
        identity_key: slot.xats_identity_key.clone(),
        name,
        team,
        source_config,
        source_native: if slot.agent == source
            || (slot.agent == "claude" && source == "claude-code")
        {
            slot.native_session_id.clone()
        } else {
            String::new()
        },
        source_pane: slot.tmux_pane.clone(),
        source_launch,
        target_tool: target.into(),
        codex_endpoint: None,
        response: None,
        target_runtime: None,
        attempt_id: None,
        controlled_identity: controlled,
        prepare_rejected: false,
    };
    pending::save(profile, &pending)?;
    prepare(profile, &mut pending)
}

fn prepare(profile: &str, pending: &mut Pending) -> Result<()> {
    if pending.response.is_some() {
        let response = super::refresh(profile, pending)?;
        ensure!(
            response.state == State::Prepared,
            "Runtime handoff completed; its state was saved, retry the requested restart"
        );
        return Ok(());
    }
    let mut expected = json!({});
    if pending.source_config.tool == "codex" && !pending.source_native.is_empty() {
        expected["thread_id"] = json!(pending.source_native);
    }
    let response = client::call(
        "prepare",
        json!({
            "protocol_version": 1, "identity_key": pending.identity_key,
            "request_id": pending.request_id, "name": pending.name, "team": pending.team,
            "source_agent_type": agent_type(&pending.source_config.tool)?,
            "target_agent_type": agent_type(&pending.target_tool)?, "expected_source": expected,
        }),
        &pending.identity_key,
    );
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            if super::legacy::resume_legacy_switch(profile, pending, &error)? {
                return Ok(());
            }
            if error
                .downcast_ref::<client::BusinessFailure>()
                .is_some_and(client::BusinessFailure::rejected_prepare)
            {
                pending.prepare_rejected = true;
                pending::save(profile, pending)?;
            }
            return Err(error);
        }
    };
    pending.validate_response(&response)?;
    ensure!(
        response.state == State::Prepared,
        "xats did not prepare the runtime handoff"
    );
    pending.response = Some(response);
    pending::save(profile, pending)
}

pub(crate) fn ensure_for_slot(
    profile: &str,
    instance: &Instance,
    slot: &AgentSlot,
    live_source: bool,
) -> Result<()> {
    if !slot.cross_agent_team || !crate::agents::is_switchable_agent(&slot.agent) {
        return Ok(());
    }
    let controlled = pending::load(profile, &instance.id, slot.slot)?.is_some();
    if (slot.native_session_id.is_empty() || controlled)
        && crate::agents::is_switchable_agent(&slot.agent)
    {
        prepare_switch(profile, instance, slot, &slot.agent, live_source)?;
    }
    Ok(())
}

pub(crate) fn begin_launch(profile: &str, id: &str, slot: i64) -> Result<()> {
    let Some(mut pending) = pending::load(profile, id, slot)? else {
        return Ok(());
    };
    if pending.response.as_ref().map(|r| r.state) != Some(State::Prepared) {
        return Ok(());
    }
    ensure!(
        pending.target_runtime.is_none(),
        "Runtime handoff commit is unresolved; complete the existing bootstrap before restarting"
    );
    pending.attempt_id = Some(uuid::Uuid::new_v4().to_string());
    pending::save(profile, &pending)
}

pub(crate) fn runtime_generation(profile: &str, id: &str, slot: i64) -> Result<Option<i64>> {
    Ok(pending::load(profile, id, slot)?
        .and_then(|pending| pending.response)
        .filter(|response| response.state == State::Committed)
        .and_then(|response| response.current)
        .and_then(|current| current.runtime_generation))
}

pub(crate) fn bootstrap_command(
    profile: &str,
    instance: &Instance,
    tool: &str,
    identity_key: Option<&str>,
    primary: bool,
    command: String,
) -> Result<String> {
    if tool == "opencode" {
        return Ok(command);
    }
    let slot = if primary {
        0
    } else {
        let store = crate::db::Store::open_with_schema(profile)?;
        let Some(slot) = store
            .read_slots_for_instance(&instance.id)?
            .into_iter()
            .find(|slot| Some(slot.xats_identity_key.as_str()) == identity_key)
        else {
            return Ok(command);
        };
        slot.slot
    };
    let Some(mut pending) = pending::load(profile, &instance.id, slot)? else {
        return Ok(command);
    };
    if pending.target_tool != tool
        || pending.response.as_ref().map(|r| r.state) != Some(State::Prepared)
    {
        return Ok(command);
    }
    ensure!(
        !shell_words::split(&command)
            .context("Invalid handoff launch command")?
            .iter()
            .any(|arg| arg == "--"),
        "Runtime handoff cannot combine its bootstrap prompt with an explicit prompt separator"
    );
    if tool == "codex" {
        pending.codex_endpoint = Some(Instance::handoff_codex_endpoint()?);
        pending::save(profile, &pending)?;
    }
    let prompt = super::complete::bootstrap_prompt(profile, &pending)?;
    Ok(format!(
        "{command} {}",
        crate::session::environment::shell_escape(&prompt)
    ))
}
