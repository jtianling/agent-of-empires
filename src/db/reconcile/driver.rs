use anyhow::{Context, Result};

use super::topology::{self, Window};
use crate::db::{AgentSlot, Store};
use crate::session::{Instance, Storage};

pub(super) fn run(profile: &str) -> Result<()> {
    let storage = Storage::new(profile)?;
    let Some(operation) = storage.try_operation_lock()? else {
        return Ok(());
    };
    let instances = storage.load()?;
    let revision = serde_json::to_value(&instances)?;
    let store = Store::open_with_schema(profile)?;
    store.conn.busy_timeout(std::time::Duration::ZERO)?;

    let listed_at = crate::db::now_unix();
    let snapshot = topology::read(profile, None)?;
    let mut work = Vec::new();
    for instance in &instances {
        let name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
        if let Some(window) = snapshot.windows.get(&name) {
            if !pending(instance) {
                let slots = store.read_slots_for_instance(&instance.id)?;
                work.push((instance, name, window, slots));
            }
        }
    }
    drop(operation);

    let mut gc_safe = true;
    for (instance, name, window, slots) in work {
        let Some(operation) = storage.try_operation_lock()? else {
            return Ok(());
        };
        let current = storage.load()?;
        if gc_safe && serde_json::to_value(&current)? != revision {
            gc_safe = false;
        }
        if !gc_safe {
            let Some(latest) = current.iter().find(|latest| latest.id == instance.id) else {
                continue;
            };
            if serde_json::to_value(latest)? != serde_json::to_value(instance)? {
                continue;
            }
        }
        // A completed operation may have replaced only the durable slot.
        if store.read_slots_for_instance(&instance.id)? != slots {
            gc_safe = false;
            continue;
        }
        if let Err(error) = reconcile_instance(
            &store,
            instance,
            &name,
            window,
            &slots,
            snapshot.sampled_before,
            profile,
        ) {
            gc_safe = false;
            tracing::debug!("Reconcile for {} deferred: {error:#}", instance.id);
        }
        drop(operation);
        std::thread::yield_now();
    }

    let Some(_operation) = storage.try_operation_lock()? else {
        return Ok(());
    };
    if gc_safe && serde_json::to_value(storage.load()?)? == revision {
        if let Some(live) = &snapshot.live {
            super::gc_orphan_pane_live(&store, live, listed_at, || {
                let current = topology::read(profile, None)?;
                anyhow::ensure!(
                    current.server == snapshot.server,
                    "Tmux server changed before GC"
                );
                current.live.context("Incomplete tmux inventory before GC")
            })?;
        }
    }
    Ok(())
}

fn pending(inst: &Instance) -> bool {
    inst.managed_primary_replacement.is_some()
        || inst
            .managed_panes
            .iter()
            .any(|pane| pane.pending_slot_transition.is_some())
}

fn reconcile_instance(
    store: &Store,
    inst: &Instance,
    name: &str,
    window: &Window,
    slots: &[AgentSlot],
    sampled_before: u64,
    profile: &str,
) -> Result<()> {
    if let Err(error) =
        super::capture_codex_bindings(store, inst, name, window, slots, sampled_before, profile)
    {
        tracing::debug!("Codex bindings for {} unavailable: {error:#}", inst.id);
    }
    // All slot and capture reads use the same SQLite version, established before
    // the first ownership check. A competing write prevents snapshot promotion.
    let tx = store.conn.unchecked_transaction()?;
    let mut confirmed = false;
    let mut confirm = || -> Result<()> {
        if !confirmed {
            anyhow::ensure!(
                window.still_owned(profile, name)?,
                "Tmux window changed during reconcile"
            );
            confirmed = true;
        }
        Ok(())
    };
    super::apply_captures(
        store,
        inst,
        &window.panes,
        window.primary.as_deref(),
        window.server_started(),
        &mut confirm,
    )?;
    super::capture_coherent_layout(store, &inst.id, window, &mut confirm)?;
    tx.commit()?;
    Ok(())
}
