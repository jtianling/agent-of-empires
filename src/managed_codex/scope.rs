use anyhow::Result;

use crate::session::Instance;

fn owns_server(instance: &Instance) -> Result<bool> {
    if instance.tool != "codex" || instance.managed_xats.is_none() {
        return Ok(false);
    }
    Ok(super::client_config(instance)?.is_some_and(|config| config.owned))
}

pub(super) fn verify(instance: &Instance) -> Result<()> {
    if owns_server(instance)? {
        let profile = instance.managed_profile();
        let (parent_id, slot) = instance
            .managed_pane_target
            .as_ref()
            .map(|target| (target.session_id.as_str(), target.slot))
            .unwrap_or((&instance.id, 0));
        let store = crate::db::Store::open_with_schema(&profile)?;
        let rows = store.read_slots_for_instance(parent_id)?;
        if let Some(row) = rows
            .iter()
            .find(|row| row.slot == slot && !row.tmux_pane.is_empty())
        {
            let session = crate::tmux::Session::generate_name(parent_id, &instance.title);
            crate::session::managed_pane::require_stored_location(
                &session,
                instance.primary_pane_id(),
                &row.tmux_pane,
                !instance.managed_primary_marked,
            )?;
        }
    }
    for pane in &instance.managed_panes {
        if pane.config.tool == "codex"
            && pane.xats.is_some()
            && owns_server(&pane.project(instance)?)?
        {
            crate::session::managed_pane::verify_stored_location(instance, pane)?;
        }
    }
    Ok(())
}
