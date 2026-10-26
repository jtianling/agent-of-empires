use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

use crate::session::{Group, Instance, ManagedPane, Storage};

pub(super) fn remove(
    storage: &Storage,
    instances: &mut Vec<Instance>,
    groups: &[Group],
    index: usize,
    owner: &str,
    id: &str,
) -> Result<Value> {
    crate::session::managed_pane::validate_pane_id(id)?;
    if id == instances[index].primary_pane_id() {
        return remove_primary(storage, instances, groups, index, owner);
    }
    ensure!(
        instances[index].managed_primary_replacement.is_none(),
        "Finish the pending primary pane removal first"
    );
    let child = instances[index]
        .managed_panes
        .iter()
        .position(|pane| pane.id == id)
        .context("Managed pane not found in this session")?;
    ensure!(
        instances[index].managed_panes[child].owner.as_deref() == Some(owner),
        super::super::coded_pane(
            anyhow::anyhow!("Pane is not owned by this manager"),
            "permission_denied",
            &instances[index].id,
            id
        )
    );
    let session =
        crate::tmux::Session::generate_name(&instances[index].id, &instances[index].title);
    let live = live(&instances[index])?;
    let record = &instances[index].managed_panes[child];
    crate::session::managed_pane::verify_stored_location(&instances[index], record)?;
    let effective = crate::session::managed_pane::effective_slot_record(
        storage.profile(),
        &instances[index],
        record,
    )?;
    validate_removal_target(storage, &instances[index], &effective, &live)?;
    if crate::session::managed_pane::recover_pending_slot(
        storage.profile(),
        &mut instances[index],
        child,
    )? {
        super::save(storage, instances, groups)?;
    }
    let record = &instances[index].managed_panes[child];
    let mut runtime = record.project(&instances[index])?;
    crate::managed_xats::prepare_release(&mut runtime)?;
    instances[index].managed_panes[child].capture_runtime(&runtime)?;
    instances[index].managed_panes[child].removing = true;
    super::save(storage, instances, groups)?;
    crate::managed_xats::release(&runtime)?;
    let record = &instances[index].managed_panes[child];
    if let Some(pane) = &record.tmux_pane {
        if live.iter().any(|entry| &entry.tmux_pane == pane) {
            crate::tmux::managed_pane::verify(&session, pane, id)?;
            crate::tmux::kill_pane_process_tree_target(pane);
            kill_if_present(&session, pane, id)?;
        }
    }
    cleanup_slot(storage, &instances[index], record)?;
    crate::managed_codex::cleanup(&runtime)?;
    crate::managed_runtimes::cleanup(&runtime)?;
    instances[index].managed_panes.remove(child);
    super::save(storage, instances, groups)?;
    Ok(
        json!({"session_id":instances[index].id,"pane_id":id,"removed":true,
        "session_removed":false,"code_resources_preserved":true}),
    )
}

fn validate_removal_target(
    storage: &Storage,
    parent: &Instance,
    pane: &ManagedPane,
    live: &[crate::tmux::managed_pane::LivePane],
) -> Result<()> {
    pane.validate()?;
    let Some(target) = &pane.tmux_pane else {
        return Ok(());
    };
    if let Some(observed) = live.iter().find(|row| &row.tmux_pane == target) {
        ensure!(
            observed.marker.as_deref() == Some(&pane.id),
            "Managed pane was replaced"
        );
    } else if let Some(location) = crate::tmux::managed_pane::locate(target)? {
        let expected = crate::tmux::Session::generate_name(&parent.id, &parent.title);
        ensure!(
            location.session == expected && location.marker.as_deref() == Some(&pane.id),
            "Managed pane moved to another session or was replaced"
        );
    }
    let store = crate::db::Store::open_with_schema(storage.profile())?;
    if let Some(slot) = pane.slot {
        if let Some(row) = store
            .read_slots_for_instance(&parent.id)?
            .iter()
            .find(|row| row.slot == slot)
        {
            ensure!(
                row.xats_identity_key == pane.identity_key
                    && row.xats_runtime_generation == pane.slot_generation
                    && (row.tmux_pane.is_empty() || row.tmux_pane == *target),
                "Managed pane slot was replaced"
            );
        }
    }
    Ok(())
}

fn cleanup_slot(storage: &Storage, parent: &Instance, pane: &ManagedPane) -> Result<()> {
    let Some(slot) = pane.slot else {
        return Ok(());
    };
    let store = crate::db::Store::open_with_schema(storage.profile())?;
    let rows = store.read_slots_for_instance(&parent.id)?;
    let target = rows
        .iter()
        .find(|row| row.slot == slot)
        .map(|row| row.tmux_pane.as_str())
        .unwrap_or("");
    store.delete_managed_pane_slot(
        &parent.id,
        slot,
        &pane.identity_key,
        target,
        pane.slot_generation,
    )
}

fn live(instance: &Instance) -> Result<Vec<crate::tmux::managed_pane::LivePane>> {
    if !crate::tmux::Session::new(&instance.id, &instance.title)?.exists() {
        return Ok(Vec::new());
    }
    crate::tmux::managed_pane::live_panes(&crate::tmux::Session::generate_name(
        &instance.id,
        &instance.title,
    ))
}

fn kill_if_present(session: &str, pane: &str, id: &str) -> Result<()> {
    if crate::tmux::managed_pane::live_panes(session)?
        .iter()
        .any(|entry| entry.tmux_pane == pane)
    {
        crate::tmux::managed_pane::verify(session, pane, id)?;
        crate::tmux::kill_pane_exact(pane)?;
    }
    Ok(())
}

fn remove_primary(
    storage: &Storage,
    instances: &mut Vec<Instance>,
    groups: &[Group],
    index: usize,
    owner: &str,
) -> Result<Value> {
    let id = instances[index].primary_pane_id().to_string();
    let live = live(&instances[index])?;
    if live.is_empty() && instances[index].managed_panes.is_empty() {
        crate::session::managed_pane::require_owned_panes(
            storage.profile(),
            &instances[index],
            owner,
        )?;
        let mut result = super::super::actions::remove(storage, instances, groups, index)?;
        result["pane_id"] = json!(id);
        result["session_removed"] = json!(true);
        return Ok(result);
    }
    let old = if let Some(pending) = &instances[index].managed_primary_replacement {
        crate::tmux::managed_pane::validate_tmux_pane(&pending.old_tmux_pane)?;
        Some(pending.old_tmux_pane.clone())
    } else {
        Some(crate::tmux::managed::primary_pane(
            &crate::tmux::Session::generate_name(&instances[index].id, &instances[index].title),
        )?)
    };
    let siblings: Vec<_> = live
        .iter()
        .filter(|pane| old.as_deref() != Some(&pane.tmux_pane))
        .collect();
    if siblings.is_empty() && instances[index].managed_primary_replacement.is_none() {
        crate::session::managed_pane::require_owned_panes(
            storage.profile(),
            &instances[index],
            owner,
        )?;
        let mut result = super::super::actions::remove(storage, instances, groups, index)?;
        result["pane_id"] = json!(id);
        result["session_removed"] = json!(true);
        return Ok(result);
    }
    let child = candidate(storage, &instances[index], owner, &live)?;
    let replacement = instances[index].managed_panes[child].id.clone();
    if instances[index].managed_primary_replacement.is_none() {
        super::primary_target(storage, instances, groups, index)?;
    }
    instances[index].managed_primary_replacement = Some(crate::session::ManagedPaneReplacement {
        pane_id: replacement.clone(),
        old_tmux_pane: old.clone().context("Missing retiring pane")?,
    });
    crate::managed_xats::prepare_release(&mut instances[index])?;
    super::save(storage, instances, groups)?;
    crate::managed_xats::release(&instances[index])?;
    complete_promotion(
        storage,
        instances,
        groups,
        index,
        child,
        &id,
        old.as_deref(),
    )?;
    Ok(
        json!({"session_id":instances[index].id,"pane_id":id,"removed":true,
        "session_removed":false,"primary_pane_id":replacement}),
    )
}

fn candidate(
    storage: &Storage,
    parent: &Instance,
    owner: &str,
    live: &[crate::tmux::managed_pane::LivePane],
) -> Result<usize> {
    let session = crate::tmux::Session::generate_name(&parent.id, &parent.title);
    for (index, pane) in parent.managed_panes.iter().enumerate() {
        if parent
            .managed_primary_replacement
            .as_ref()
            .is_some_and(|pending| pending.pane_id != pane.id)
            || pane.owner.as_deref() != Some(owner)
            || pane.removing
        {
            continue;
        }
        let Some(target) = &pane.tmux_pane else {
            continue;
        };
        if !live
            .iter()
            .any(|entry| &entry.tmux_pane == target && entry.marker.as_deref() == Some(&pane.id))
        {
            continue;
        }
        if parent
            .managed_primary_replacement
            .as_ref()
            .map(|pending| &pending.pane_id)
            != Some(&pane.id)
        {
            crate::session::managed_pane::verify_binding(storage.profile(), parent, pane)?;
        } else {
            let rows = crate::db::Store::open_with_schema(storage.profile())?
                .read_slots_for_instance(&parent.id)?;
            ensure!(
                rows.iter()
                    .any(|row| (Some(row.slot) == pane.slot || row.slot == 0)
                        && row.xats_identity_key == pane.identity_key
                        && row.tmux_pane == *target
                        && row.xats_runtime_generation == pane.slot_generation),
                "Pending promotion candidate slot changed"
            );
        }
        if crate::tmux::managed_pane::is_ready(&session, target, &pane.id, pane.slot_generation)? {
            return Ok(index);
        }
    }
    anyhow::bail!("No owned launch-ready sibling can become primary; retry when ready")
}

#[allow(clippy::too_many_arguments)]
fn complete_promotion(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
    child: usize,
    old_id: &str,
    old_pane: Option<&str>,
) -> Result<()> {
    let parent = &instances[index];
    let session = crate::tmux::Session::generate_name(&parent.id, &parent.title);
    let mut replacement = parent.managed_panes[child].clone();
    let store = crate::db::Store::open_with_schema(storage.profile())?;
    store.promote_managed_pane_slot(
        &parent.id,
        &session,
        parent.xats_identity_key.as_deref().unwrap_or(""),
        old_pane.unwrap_or(""),
        &replacement,
    )?;
    if let Some(target) = old_pane {
        if live(parent)?.iter().any(|entry| entry.tmux_pane == target) {
            crate::tmux::managed_pane::verify(&session, target, old_id)?;
            crate::tmux::kill_pane_process_tree_target(target);
            kill_if_present(&session, target, old_id)?;
        }
        store.delete_retired_pane_records(&parent.id, target)?;
    }
    let native = store
        .read_slots_for_instance(&parent.id)?
        .into_iter()
        .find(|row| row.slot == 0)
        .context("Promoted primary slot disappeared")?
        .native_session_id;
    if !native.is_empty() {
        replacement.native_session_id = Some(native);
    }
    crate::tmux::set_agent_pane_id(
        &session,
        replacement
            .tmux_pane
            .as_deref()
            .context("Missing promotion target")?,
    )?;
    crate::managed_codex::cleanup(parent)?;
    crate::managed_runtimes::cleanup(parent)?;
    replacement.promote_into(&mut instances[index])?;
    instances[index].managed_panes.remove(child);
    instances[index].managed_primary_replacement = None;
    super::save(storage, instances, groups)
}
