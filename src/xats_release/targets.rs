use serde::Serialize;

use crate::db::AgentSlot;
use crate::session::Instance;
use crate::tmux::inventory::Pane;
use crate::xats_pane_lookup::PaneTarget;

pub(super) const MAX_TARGETS: usize = 16;

#[derive(Clone, Serialize)]
pub(super) struct Target {
    pub tmux: PaneTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opencode_session_id: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub allow_dead_if_identity_unclaimed: bool,
}

pub(super) struct Plan {
    pub targets: Vec<Target>,
    pub missing_keys: Vec<String>,
    pub requires_daemon: bool,
}

pub(super) fn plan(instance: &Instance, slots: &[AgentSlot], panes: &[Pane]) -> Plan {
    let targets = panes
        .iter()
        .filter(|pane| !managed(instance, slots, pane))
        .map(|pane| target(instance, slots, pane))
        .collect::<Vec<_>>();
    let keys = ordinary_keys(instance, slots);
    let missing_keys = keys
        .iter()
        .filter(|key| {
            !targets
                .iter()
                .any(|target| target.identity_key.as_deref() == Some(key.as_str()))
        })
        .cloned()
        .collect();
    let requires_daemon = !keys.is_empty()
        || (instance.managed_xats.is_none() && instance.primary_pane_config().cross_agent_team)
        || slots
            .iter()
            .any(|slot| !managed_slot(instance, slot.slot) && slot.cross_agent_team);
    Plan {
        targets,
        missing_keys,
        requires_daemon,
    }
}

fn target(instance: &Instance, slots: &[AgentSlot], pane: &Pane) -> Target {
    let slot = slots
        .iter()
        .find(|slot| slot.tmux_pane == pane.target.pane_id);
    let identity_key = slot
        .and_then(|slot| nonempty(&slot.xats_identity_key))
        .or_else(|| {
            (pane.primary && slot.map_or(true, |slot| slot.slot == 0))
                .then(|| instance.xats_identity_key.clone())
                .flatten()
        })
        .filter(|key| !key.is_empty());
    let opencode_session_id = slot
        .filter(|slot| slot.agent == "opencode")
        .and_then(|slot| nonempty(&slot.native_session_id));
    let allow_dead_if_identity_unclaimed =
        !crate::session::managed_pane::has_managed_state(instance)
            && pane.dead
            && pane.root_pid == 0
            && pane.tty.is_empty()
            && slot.is_some_and(|slot| !slot.xats_identity_key.is_empty())
            && opencode_session_id.is_none();
    Target {
        tmux: pane.target.clone(),
        identity_key,
        opencode_session_id,
        allow_dead_if_identity_unclaimed,
    }
}

fn managed(instance: &Instance, slots: &[AgentSlot], pane: &Pane) -> bool {
    (pane.primary && instance.managed_xats.is_some())
        || slots
            .iter()
            .any(|slot| slot.tmux_pane == pane.target.pane_id && managed_slot(instance, slot.slot))
        || instance.managed_panes.iter().any(|record| {
            record.xats.is_some()
                && (record.tmux_pane.as_deref() == Some(&pane.target.pane_id)
                    || pane.marker.as_deref() == Some(&record.id))
        })
}

fn managed_slot(instance: &Instance, slot: i64) -> bool {
    (slot == 0 && instance.managed_xats.is_some())
        || instance
            .managed_panes
            .iter()
            .any(|pane| pane.slot == Some(slot) && pane.xats.is_some())
}

fn ordinary_keys(instance: &Instance, slots: &[AgentSlot]) -> Vec<String> {
    let mut keys = slots
        .iter()
        .filter(|slot| !managed_slot(instance, slot.slot))
        .filter_map(|slot| nonempty(&slot.xats_identity_key))
        .collect::<Vec<_>>();
    if instance.managed_xats.is_none()
        && !slots
            .iter()
            .any(|slot| slot.slot == 0 && !slot.xats_identity_key.is_empty())
    {
        keys.extend(
            instance
                .xats_identity_key
                .clone()
                .filter(|key| !key.is_empty()),
        );
    }
    keys.sort();
    keys.dedup();
    keys
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn is_false(value: &bool) -> bool {
    !value
}

#[cfg(test)]
mod tests;
