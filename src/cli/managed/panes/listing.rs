use anyhow::Result;
use serde_json::{json, Value};

use crate::session::{Instance, Storage};
use crate::tmux::managed_pane::LivePane;

pub(super) fn list(storage: &Storage, instance: &Instance, owner: &str) -> Result<Value> {
    let name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
    let live = if crate::tmux::Session::new(&instance.id, &instance.title)?.exists() {
        crate::tmux::managed_pane::live_panes(&name)?
    } else {
        Vec::new()
    };
    let slots = crate::db::Store::open_with_schema(storage.profile())?
        .read_slots_for_instance(&instance.id)?;
    let pinned = if live.is_empty() {
        None
    } else {
        match crate::tmux::managed::primary_pane(&name) {
            Ok(pane) => Some(pane),
            Err(error) => {
                tracing::warn!("Cannot identify exact primary pane: {error:#}");
                None
            }
        }
    };
    let primary = slots.iter().find(|slot| slot.slot == 0).and_then(|slot| {
        live.iter().find(|pane| {
            pane.tmux_pane == slot.tmux_pane
                && pinned.as_deref() == Some(&pane.tmux_pane)
                && (!instance.managed_primary_marked
                    || pane.marker.as_deref() == Some(instance.primary_pane_id()))
        })
    });
    let mut result = vec![entry(
        instance,
        instance.primary_pane_id(),
        Some(0),
        primary,
        &instance.tool,
        &instance.project_path,
        instance.managed_owner.as_deref(),
        owner,
        true,
    )];
    if instance.managed_owner.as_deref() == Some(owner) {
        result[0]["xats"] = crate::managed_xats::status_report(instance);
    }
    for pane in &instance.managed_panes {
        pane.validate()?;
        let observed = pane.tmux_pane.as_ref().and_then(|target| {
            live.iter().find(|entry| {
                &entry.tmux_pane == target && entry.marker.as_deref() == Some(&pane.id)
            })
        });
        let mut metadata = entry(
            instance,
            &pane.id,
            pane.slot,
            observed,
            &pane.config.tool,
            &pane.config.working_dir,
            pane.owner.as_deref(),
            owner,
            false,
        );
        metadata["removing"] = json!(pane.removing);
        if instance.managed_owner.as_deref() == Some(owner) && pane.owner.as_deref() == Some(owner)
        {
            metadata["xats"] = crate::managed_xats::status_report(&pane.project(instance)?);
        }
        result.push(metadata);
    }
    for pane in &live {
        if result
            .iter()
            .any(|entry| entry["tmux_pane"].as_str() == Some(&pane.tmux_pane))
        {
            continue;
        }
        let slot = slots.iter().find(|row| row.tmux_pane == pane.tmux_pane);
        result.push(json!({"session_id":instance.id, "pane_id":null,
            "slot":slot.map(|row| row.slot), "tmux_pane":pane.tmux_pane,
            "index":pane.index, "left":pane.left, "top":pane.top,
            "primary":false,"managed":false,"owned":false,"owner":null,
            "live":true, "tool":slot.map(|row| &row.agent)}));
    }
    Ok(json!(result))
}

#[allow(clippy::too_many_arguments)]
fn entry(
    parent: &Instance,
    id: &str,
    slot: Option<i64>,
    live: Option<&LivePane>,
    tool: &str,
    path: &str,
    pane_owner: Option<&str>,
    owner: &str,
    primary: bool,
) -> Value {
    json!({"session_id":parent.id,"pane_id":id,"slot":slot,
        "tmux_pane":live.map(|pane| &pane.tmux_pane),
        "index":live.map(|pane| pane.index),"left":live.map(|pane| pane.left),
        "top":live.map(|pane| pane.top),"primary":primary,"managed":pane_owner.is_some(),
        "owned":parent.managed_owner.as_deref()==Some(owner) && pane_owner==Some(owner)
            && (live.is_some() || !primary),
        "owner":pane_owner,"tool":tool,"path":path,"live":live.is_some()})
}
