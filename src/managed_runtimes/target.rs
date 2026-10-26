use anyhow::{ensure, Context, Result};

use crate::session::{managed_pane, Instance, Storage};

pub(crate) fn load_pane(
    profile: &str,
    session_id: &str,
    pane_id: &str,
    slot: i64,
    launch_id: Option<&str>,
    tool: &str,
) -> Result<Instance> {
    managed_pane::validate_pane_id(session_id)?;
    managed_pane::validate_pane_id(pane_id)?;
    let mut parent = Storage::new(profile)?
        .load()?
        .into_iter()
        .find(|instance| instance.id == session_id)
        .context("Managed pane parent session is missing")?;
    parent.source_profile = profile.to_string();
    ensure!(
        parent.managed_primary_replacement.is_none(),
        "Managed primary replacement is incomplete"
    );
    if slot == 0 {
        return load_primary(profile, parent, pane_id, launch_id, tool);
    }
    let pane = parent
        .managed_panes
        .iter()
        .find(|pane| pane.id == pane_id)
        .context("Managed pane identity is missing")?;
    ensure!(
        !pane.removing && pane.slot == Some(slot),
        "Managed pane launch slot is stale"
    );
    ensure!(
        pane.config.tool == tool,
        "Managed pane launch tool mismatch"
    );
    let live_pane = managed_pane::verify_binding(profile, &parent, pane)?;
    validate_process_pane(&live_pane, std::env::var("TMUX_PANE").ok().as_deref())?;
    let projected = pane.project(&parent)?;
    match &projected.managed_xats {
        Some(identity) => {
            let claim = identity
                .claim
                .as_ref()
                .context("Managed pane has no claim")?;
            ensure!(
                launch_id == Some(claim.fence.instance_id.as_str()),
                "Managed pane launch fence is stale"
            );
        }
        None => ensure!(launch_id.is_none(), "Managed pane identity was removed"),
    }
    Ok(projected)
}

fn load_primary(
    profile: &str,
    mut parent: Instance,
    pane_id: &str,
    launch_id: Option<&str>,
    tool: &str,
) -> Result<Instance> {
    ensure!(
        parent.primary_pane_id() == pane_id && parent.tool == tool,
        "Managed primary identity changed before launch"
    );
    let session = crate::tmux::Session::generate_name(&parent.id, &parent.title);
    let pane = crate::tmux::managed::primary_pane(&session)?;
    crate::tmux::managed_pane::verify(&session, &pane, pane_id)?;
    validate_process_pane(&pane, std::env::var("TMUX_PANE").ok().as_deref())?;
    let row = crate::db::Store::open_with_schema(profile)?
        .read_slots_for_instance(&parent.id)?
        .into_iter()
        .find(|row| row.slot == 0)
        .context("Managed primary slot is missing")?;
    ensure!(
        row.tmux_pane == pane
            && parent.xats_identity_key.as_deref().unwrap_or_default() == row.xats_identity_key,
        "Managed primary slot binding changed"
    );
    let claim = parent
        .managed_xats
        .as_ref()
        .and_then(|identity| identity.claim.as_ref());
    ensure!(
        launch_id == claim.map(|claim| claim.fence.instance_id.as_str()),
        "Managed primary launch fence is stale"
    );
    let session_id = parent.id.clone();
    parent.id = pane_id.to_string();
    parent.managed_primary_id = None;
    parent.managed_panes.clear();
    parent.managed_pane_target = Some(crate::session::ManagedPaneTarget {
        session_id,
        pane_id: pane_id.to_string(),
        slot: 0,
    });
    Ok(parent)
}

fn validate_process_pane(recorded: &str, current: Option<&str>) -> Result<()> {
    ensure!(
        current == Some(recorded),
        "Managed launch is not running in its recorded pane"
    );
    Ok(())
}

pub(crate) fn mark_ready(profile: &str, instance: &Instance) -> Result<()> {
    let Some(target) = &instance.managed_pane_target else {
        return Ok(());
    };
    let current = load_pane(
        profile,
        &target.session_id,
        &target.pane_id,
        target.slot,
        instance
            .managed_xats
            .as_ref()
            .and_then(|identity| identity.claim.as_ref())
            .map(|claim| claim.fence.instance_id.as_str()),
        &instance.tool,
    )?;
    let parent = Storage::new(profile)?
        .load()?
        .into_iter()
        .find(|parent| parent.id == target.session_id)
        .context("Managed parent disappeared")?;
    let row = crate::db::Store::open_with_schema(profile)?
        .read_slots_for_instance(&target.session_id)?
        .into_iter()
        .find(|row| row.slot == target.slot)
        .context("Managed slot disappeared")?;
    ensure!(
        current.xats_identity_key.as_deref().unwrap_or_default() == row.xats_identity_key,
        "Managed slot identity changed"
    );
    let session = crate::tmux::Session::generate_name(&parent.id, &parent.title);
    crate::tmux::managed_pane::mark_ready(
        &session,
        &row.tmux_pane,
        &target.pane_id,
        row.xats_runtime_generation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_must_be_inside_the_exact_recorded_pane() {
        assert!(validate_process_pane("%3", Some("%3")).is_ok());
        for current in [None, Some("%4"), Some("%30"), Some("")] {
            assert!(validate_process_pane("%3", current).is_err());
        }
    }
}
