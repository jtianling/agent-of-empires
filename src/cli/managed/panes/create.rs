use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

use crate::cli::managed::pane_args::CreateArgs;
use crate::session::{Group, Instance, ManagedPane, PaneConfig, RestartMode, Storage};

pub(super) fn create(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
    owner: &str,
    args: CreateArgs,
) -> Result<Value> {
    ensure!(
        instances[index].managed_primary_replacement.is_none(),
        "Finish the pending primary pane removal first"
    );
    ensure!(
        !instances[index].is_sandboxed(),
        "Managed pane creation requires a host session"
    );
    let config = config(&instances[index], &args)?;
    let xats = xats(&args)?;
    let name = crate::tmux::Session::generate_name(&instances[index].id, &instances[index].title);
    ensure!(
        crate::tmux::Session::new(&instances[index].id, &instances[index].title)?.exists(),
        "Parent session is not running"
    );
    let live = crate::tmux::managed_pane::live_panes(&name)?;
    ensure!(
        live.len() < crate::db::reconcile::MAX_AGENT_PANES,
        "Session already has the maximum number of panes"
    );
    super::primary_target(storage, instances, groups, index)?;
    let pane = ManagedPane::new(owner.to_string(), config, xats)?;
    ensure!(
        pane.id != instances[index].primary_pane_id()
            && !instances[index]
                .managed_panes
                .iter()
                .any(|existing| existing.id == pane.id),
        "Generated pane ID collided with an existing pane; retry creation"
    );
    let id = pane.id.clone();
    let child = instances[index].managed_panes.len();
    instances[index].managed_panes.push(pane);
    super::save(storage, instances, groups)?;
    let result = (|| {
        start(storage, instances, groups, index, child, &name, &live)?;
        let pane = &instances[index].managed_panes[child];
        Ok(json!({"session_id":instances[index].id,"pane_id":pane.id,
            "slot":pane.slot,"tmux_pane":pane.tmux_pane,"owner":pane.owner,
            "xats":crate::managed_xats::status(&pane.project(&instances[index])?)?}))
    })();
    result.map_err(|error| {
        super::super::coded_pane(error, "pane_create_incomplete", &instances[index].id, &id)
    })
}

fn config(parent: &Instance, args: &CreateArgs) -> Result<PaneConfig> {
    ensure!(
        crate::agents::get_agent(&args.tool).is_some(),
        "Unknown agent tool"
    );
    let directory = args
        .path
        .clone()
        .unwrap_or_else(|| parent.project_path.clone().into())
        .canonicalize()
        .context("Resolving pane working directory")?;
    ensure!(directory.is_dir(), "Pane working directory must exist");
    let path = directory.to_str().context("Pane path must be UTF-8")?;
    let config = PaneConfig::new(&args.tool, path, false, args.xats_name.is_some());
    config.validate()?;
    Ok(config)
}

fn xats(args: &CreateArgs) -> Result<Option<crate::managed_xats::ManagedXats>> {
    match (&args.xats_name, &args.xats_team) {
        (None, None) => Ok(None),
        (Some(name), Some(team)) => {
            ensure!(
                matches!(args.tool.as_str(), "codex" | "claude" | "opencode"),
                "Managed xats pane launch is unsupported for this tool"
            );
            Ok(Some(crate::managed_xats::ManagedXats::new(
                name.clone(),
                team.clone(),
            )?))
        }
        _ => anyhow::bail!("Specify both xats name and team"),
    }
}

fn start(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
    child: usize,
    session: &str,
    live: &[crate::tmux::managed_pane::LivePane],
) -> Result<()> {
    let store = crate::db::Store::open_with_schema(storage.profile())?;
    let record = &instances[index].managed_panes[child];
    let (slot, prepared) = store.reserve_managed_pane_slot(
        &instances[index].id,
        &record.config,
        &record.identity_key,
        &live
            .iter()
            .map(|pane| pane.tmux_pane.clone())
            .collect::<Vec<_>>(),
    )?;
    let record = &mut instances[index].managed_panes[child];
    record.slot = Some(slot);
    record.slot_generation = prepared.generation;
    persist_reservation(
        storage, instances, groups, index, child, &store, slot, &prepared,
    )?;
    let command =
        instances[index].prepare_managed_pane_runtime(storage.profile(), child, RestartMode::Fresh);
    super::save(storage, instances, groups)?;
    let command = command?;
    spawn(
        storage, instances, groups, index, child, session, &store, &command,
    )
}

#[allow(clippy::too_many_arguments)]
fn persist_reservation(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
    child: usize,
    store: &crate::db::Store,
    slot: i64,
    receipt: &crate::db::PreparedSlotRuntime,
) -> Result<()> {
    if let Err(error) = super::save(storage, instances, groups) {
        let record = &instances[index].managed_panes[child];
        let cleanup = store.rollback_unbound_exact_session_slot(
            &instances[index].id,
            &record.config.tool,
            slot,
            receipt.generation,
            &receipt.xats_identity_key,
            &receipt.native_session_id,
        );
        return match cleanup {
            Ok(()) => {
                let record = &mut instances[index].managed_panes[child];
                record.slot = None;
                record.slot_generation = 0;
                Err(error
                    .context("Saving managed pane reservation failed; reservation rolled back"))
            }
            Err(cleanup) => Err(anyhow::anyhow!(
                "{error:#}; exact managed pane reservation rollback failed: {cleanup:#}"
            )),
        };
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
    child: usize,
    session: &str,
    store: &crate::db::Store,
    command: &str,
) -> Result<()> {
    let record = &instances[index].managed_panes[child];
    let placeholder =
        crate::session::environment::shell_escape(&crate::session::environment::user_shell());
    let pane =
        crate::tmux::split_window_right(session, &record.config.working_dir, &placeholder, true)?;
    instances[index].managed_panes[child].tmux_pane = Some(pane.clone());
    let result = finish_spawn(
        storage, instances, groups, index, child, session, store, command, &pane,
    );
    if let Err(error) = result {
        return match rollback_placeholder(
            storage, instances, groups, index, child, session, store, &pane,
        ) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(anyhow::anyhow!(
                "{error:#}; pane rollback failed: {cleanup:#}"
            )),
        };
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_spawn(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
    child: usize,
    session: &str,
    store: &crate::db::Store,
    command: &str,
    pane: &str,
) -> Result<()> {
    super::save(storage, instances, groups)?;
    let record = &instances[index].managed_panes[child];
    crate::tmux::managed_pane::mark(session, pane, &record.id)?;
    let slot = record.slot.context("Missing prepared pane slot")?;
    let prepared = store
        .read_slots_for_instance(&instances[index].id)?
        .into_iter()
        .find(|row| row.slot == slot)
        .context("Prepared pane slot disappeared")?;
    ensure!(
        prepared.xats_identity_key == record.identity_key
            && prepared.xats_runtime_generation == record.slot_generation
            && prepared.tmux_pane.is_empty(),
        "Prepared slot changed before binding"
    );
    let result = store.bind_prepared_slot_pane(
        &instances[index].id,
        &record.config.tool,
        slot,
        record.slot_generation,
        &record.identity_key,
        &prepared.native_session_id,
        pane,
        crate::db::now_unix(),
    );
    super::save(storage, instances, groups)?;
    result?;
    let record = &instances[index].managed_panes[child];
    crate::tmux::respawn_pane_target(
        pane,
        command,
        &record.config.working_dir,
        record.config.tool != "shell",
    )?;
    if record.config.tool != "opencode" && record.xats.is_none() {
        crate::tmux::managed_pane::mark_ready(session, pane, &record.id, record.slot_generation)?;
    }
    instances[index].auto_confirm_launched_pane(pane, &record.config);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn rollback_placeholder(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
    child: usize,
    session: &str,
    store: &crate::db::Store,
    target: &str,
) -> Result<()> {
    let record = &instances[index].managed_panes[child];
    if let Some(location) = crate::tmux::managed_pane::locate(target)? {
        ensure!(
            location.session == session
                && location
                    .marker
                    .as_deref()
                    .map_or(true, |marker| marker == record.id),
            "Created pane changed before rollback"
        );
        crate::tmux::kill_pane_exact(target)?;
    }
    let slot = record.slot.context("Created pane has no reserved slot")?;
    let rows = store.read_slots_for_instance(&instances[index].id)?;
    if let Some(row) = rows.iter().find(|row| row.slot == slot) {
        ensure!(
            row.tmux_pane.is_empty() || row.tmux_pane == target,
            "Created slot changed before rollback"
        );
        store.delete_managed_pane_slot(
            &instances[index].id,
            slot,
            &record.identity_key,
            &row.tmux_pane,
            record.slot_generation,
        )?;
    }
    let record = &mut instances[index].managed_panes[child];
    record.tmux_pane = None;
    record.slot = None;
    record.slot_generation = 0;
    super::save(storage, instances, groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn failed_json_save_rolls_back_only_its_exact_slot_reservation() {
        let directory = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(directory.path());
        let storage = Storage::new("pane-reservation-failure").unwrap();
        let store = crate::db::Store::open_with_schema(storage.profile()).unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        let mut parent = Instance::new("parent", "/tmp");
        parent.managed_owner = Some(owner.clone());
        parent.managed_panes.push(
            ManagedPane::new(owner, PaneConfig::new("shell", "/tmp", false, false), None).unwrap(),
        );
        let mut instances = vec![parent];
        super::super::save(&storage, &instances, &[]).unwrap();
        let record = &instances[0].managed_panes[0];
        let (slot, receipt) = store
            .reserve_managed_pane_slot(&instances[0].id, &record.config, &record.identity_key, &[])
            .unwrap();
        let (other, _) = store
            .reserve_managed_pane_slot(&instances[0].id, &record.config, "unrelated-receipt", &[])
            .unwrap();
        let concurrent = Storage::new(storage.profile()).unwrap();
        let mut updated = concurrent.load().unwrap();
        updated[0].title = "concurrently-edited".into();
        concurrent.save(&updated).unwrap();
        instances[0].managed_panes[0].slot = Some(slot);
        instances[0].managed_panes[0].slot_generation = receipt.generation;
        let error =
            persist_reservation(&storage, &mut instances, &[], 0, 0, &store, slot, &receipt)
                .unwrap_err();
        assert!(format!("{error:#}").contains("reservation rolled back"));
        let rows = store.read_slots_for_instance(&instances[0].id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slot, other);
        assert_eq!(rows[0].xats_identity_key, "unrelated-receipt");
        let durable = concurrent.load().unwrap();
        assert_eq!(durable[0].title, "concurrently-edited");
        assert!(durable[0].managed_panes[0].slot.is_none());
        assert!(instances[0].managed_panes[0].slot.is_none());
    }
}
