mod create;
mod listing;
mod remove;

use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

use super::pane_args::PaneCommand;
use crate::session::{Group, GroupTree, Instance, Storage};

pub(super) fn run(
    storage: &Storage,
    instances: &mut Vec<Instance>,
    groups: &[Group],
    owner: &str,
    command: PaneCommand,
) -> Result<Value> {
    let session_id = command.session_id().to_string();
    let index = super::exact_index(instances, &session_id)?;
    if !matches!(command, PaneCommand::List(_)) {
        super::owned_index(instances, &session_id, owner)?;
    }
    if !matches!(command, PaneCommand::List(_) | PaneCommand::Remove(_)) {
        ensure!(
            instances[index].managed_primary_replacement.is_none(),
            "Finish the pending primary pane removal first"
        );
    }
    let pane_id = match &command {
        PaneCommand::Send(args) => Some(args.pane_id.clone()),
        PaneCommand::Capture(args) => Some(args.pane_id.clone()),
        PaneCommand::Remove(args) => Some(args.pane_id.clone()),
        _ => None,
    };
    let result = (|| match command {
        PaneCommand::List(_) => listing::list(storage, &instances[index], owner),
        PaneCommand::Create(args) => create::create(storage, instances, groups, index, owner, args),
        PaneCommand::Send(args) => {
            ensure!(
                !args.message.trim().is_empty()
                    && args.message.len() <= 65536
                    && !args.message.contains('\0'),
                "Invalid message"
            );
            let pane = owned_target(storage, instances, groups, index, owner, &args.pane_id)?;
            crate::tmux::submit_text_to_pane_target(&pane, &args.message)?;
            Ok(json!({"session_id": session_id, "pane_id": args.pane_id, "sent": true}))
        }
        PaneCommand::Capture(args) => {
            let pane = owned_target(storage, instances, groups, index, owner, &args.pane_id)?;
            capture(&session_id, &args.pane_id, &pane, args.lines)
        }
        PaneCommand::Remove(args) => {
            remove::remove(storage, instances, groups, index, owner, &args.pane_id)
        }
    })();
    result.map_err(|error| {
        if error.downcast_ref::<super::Failure>().is_some() {
            error
        } else {
            match pane_id {
                Some(id) => super::coded_pane(error, "pane_operation_failed", &session_id, &id),
                None => super::coded(error, "pane_operation_failed", Some(&session_id)),
            }
        }
    })
}

fn owned_target(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
    owner: &str,
    id: &str,
) -> Result<String> {
    crate::session::managed_pane::validate_pane_id(id)?;
    let instance = &instances[index];
    if id == instance.primary_pane_id() {
        return primary_target(storage, instances, groups, index);
    }
    let pane = instance
        .managed_panes
        .iter()
        .find(|pane| pane.id == id)
        .context("Managed pane not found in this session")?;
    ensure!(
        pane.owner.as_deref() == Some(owner),
        super::coded_pane(
            anyhow::anyhow!("Pane is not owned by this manager"),
            "permission_denied",
            &instance.id,
            id
        )
    );
    ensure!(!pane.removing, "Managed pane is being removed");
    crate::session::managed_pane::verify_binding(storage.profile(), instance, pane)
}

pub(super) fn primary_target(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
) -> Result<String> {
    let instance = &mut instances[index];
    let session = crate::tmux::Session::generate_name(&instance.id, &instance.title);
    let pane = crate::tmux::managed::primary_pane(&session)?;
    if !instance.managed_primary_marked {
        let rows = crate::db::Store::open_with_schema(storage.profile())?
            .read_slots_for_instance(&instance.id)?;
        ensure!(
            rows.iter()
                .any(|row| row.slot == 0 && row.tmux_pane == pane),
            "Primary slot does not match the live session"
        );
        crate::tmux::managed_pane::mark(&session, &pane, instance.primary_pane_id())?;
        instance.managed_primary_marked = true;
        save(storage, instances, groups)?;
    }
    crate::tmux::managed_pane::verify(&session, &pane, instances[index].primary_pane_id())?;
    Ok(pane)
}

fn capture(session: &str, id: &str, pane: &str, lines: u32) -> Result<Value> {
    let output = crate::tmux::tmux_command()
        .args([
            "capture-pane",
            "-t",
            pane,
            "-p",
            "-e",
            "-S",
            &format!("-{lines}"),
        ])
        .output()
        .context("Capturing exact managed pane")?;
    ensure!(
        output.status.success(),
        "Capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(json!({"session_id": session, "pane_id": id,
        "content": String::from_utf8(output.stdout).context("Invalid pane output encoding")?}))
}

fn save(storage: &Storage, instances: &[Instance], groups: &[Group]) -> Result<()> {
    storage.save_with_groups(instances, &GroupTree::new_with_groups(instances, groups))
}
