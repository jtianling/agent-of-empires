use std::collections::HashSet;

use anyhow::Result;

use crate::db::Store;
use crate::session::{Instance, Storage};

pub(super) fn run(profile: &str) -> Result<()> {
    let storage = Storage::new(profile)?;
    let Some(operation) = storage.try_operation_lock()? else {
        return Ok(());
    };
    let instances = storage.load()?;
    let revision = serde_json::to_value(&instances)?;
    drop(operation);

    let store = Store::open_with_schema(profile)?;
    store.conn.busy_timeout(std::time::Duration::ZERO)?;

    let listed_at = crate::db::now_unix();
    let mut live_panes = HashSet::new();
    let mut gc_safe = true;
    for original in &instances {
        let Some(operation) = storage.try_operation_lock()? else {
            return Ok(());
        };
        let current = storage.load()?;
        if gc_safe && serde_json::to_value(&current)? != revision {
            gc_safe = false;
        }
        if let Some(instance) = current.iter().find(|instance| instance.id == original.id) {
            reconcile_instance(&store, instance, &mut live_panes);
        }
        drop(operation);
        std::thread::yield_now();
    }

    let Some(_operation) = storage.try_operation_lock()? else {
        return Ok(());
    };
    // A concurrent edit can create unobserved panes; retry GC on a stable pass.
    if gc_safe && serde_json::to_value(storage.load()?)? == revision {
        super::gc_orphan_pane_live(&store, &live_panes, listed_at);
    }
    Ok(())
}

fn reconcile_instance(store: &Store, inst: &Instance, live_panes: &mut HashSet<String>) {
    let session_name = crate::tmux::Session::generate_name(&inst.id, &inst.title);
    let panes = super::list_session_panes(&session_name);
    if panes.is_empty() {
        return;
    }
    live_panes.extend(panes.iter().map(|(_, id)| id.clone()));
    if inst.managed_primary_replacement.is_some()
        || inst
            .managed_panes
            .iter()
            .any(|pane| pane.pending_slot_transition.is_some())
    {
        return;
    }
    let primary = crate::tmux::get_agent_pane_id(&session_name);
    if let Err(error) = super::capture_codex_bindings(store, inst, &panes, primary.as_deref()) {
        tracing::debug!("Codex bindings for {} unavailable: {error:#}", inst.id);
    }
    if let Err(error) = super::reconcile_session(store, inst, &panes, primary.as_deref()) {
        tracing::debug!("reconcile: session {} failed: {error:#}", inst.id);
    }
    super::capture_coherent_layout(store, &inst.id, &session_name, &panes);
}
