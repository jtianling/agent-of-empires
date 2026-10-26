use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

use super::args::{CreateArgs, ManagedCommand};
use crate::session::{builder, Group, GroupTree, Instance, PaneDraft, Status, Storage};

pub(super) fn target_id(command: &ManagedCommand) -> &str {
    match command {
        ManagedCommand::Start(args)
        | ManagedCommand::Stop(args)
        | ManagedCommand::Restart(args)
        | ManagedCommand::Remove(args) => &args.id,
        ManagedCommand::Rename(args) => &args.id,
        ManagedCommand::Send(args) => &args.id,
        ManagedCommand::Capture(args) => &args.id,
        _ => unreachable!(),
    }
}

pub(super) fn create(
    storage: &Storage,
    mut instances: Vec<Instance>,
    groups: Vec<Group>,
    owner: &str,
    args: CreateArgs,
) -> Result<Value> {
    let mut instance = build(storage.profile(), owner, &args)
        .map_err(|error| super::coded(error, "invalid_input", None))?;
    let id = instance.id.clone();
    ensure_unique_name(&instances, &instance)?;
    instances.push(instance.clone());
    save(storage, &instances, &groups)?;
    let result = (|| {
        crate::managed_xats::prepare(&mut instance)?;
        replace(storage, &mut instances, &groups, &instance)?;
        if args.launch {
            let started = instance.start_with_size(crate::terminal::get_size());
            replace(storage, &mut instances, &groups, &instance)?;
            started?;
        }
        let mut output = super::metadata(&instance, owner);
        output["xats"] = crate::managed_xats::status(&instance)?;
        Ok(output)
    })();
    result.map_err(|error| super::coded(error, "create_incomplete", Some(&id)))
}

fn build(profile: &str, owner: &str, args: &CreateArgs) -> Result<Instance> {
    validate_title(&args.title)?;
    if !args.group.is_empty() {
        crate::session::validate_group_path(&args.group)?;
    }
    let path = args
        .path
        .canonicalize()
        .context("Resolving project directory")?;
    ensure!(path.is_dir(), "Project path must be a directory");
    let path = path
        .to_str()
        .context("Project path must be UTF-8")?
        .to_string();
    let xats = match (&args.xats_name, &args.xats_team) {
        (Some(name), Some(team)) => {
            ensure!(
                args.tool != "kimi",
                "Managed xats launch is not supported for kimi"
            );
            ensure!(
                Instance::supports_cross_agent_team_tool(&args.tool),
                "Tool does not support xats"
            );
            Some(crate::managed_xats::ManagedXats::new(
                name.clone(),
                team.clone(),
            )?)
        }
        (None, None) => None,
        _ => anyhow::bail!("Specify both --xats-name and --xats-team"),
    };
    let config = crate::session::resolve_config(profile)?;
    let mut instance = builder::build_instance(
        builder::InstanceParams {
            title: args.title.trim().to_string(),
            group: args.group.clone(),
            primary: PaneDraft {
                tool: args.tool.clone(),
                path,
                yolo_mode: false,
                cross_agent_team: xats.is_some(),
                worktree: Default::default(),
            },
            sandbox: false,
            sandbox_image: String::new(),
            extra_env: Vec::new(),
            extra_args: String::new(),
            command_override: String::new(),
            cross_agent_team_channel: config.session.cross_agent_team_channel,
        },
        &[],
        profile,
    )?
    .instance;
    if args.tool == "codex" && xats.is_some() {
        let default_command = crate::agents::get_agent("codex")
            .context("Codex is not registered")?
            .binary;
        ensure!(
            instance.extra_args.is_empty()
                && (instance.command.is_empty() || instance.command == default_command),
            "Managed Codex xats does not support command overrides or extra arguments"
        );
    }
    instance.status = Status::Stopped;
    instance.managed_owner = Some(owner.to_string());
    instance.managed_xats = xats;
    if instance.managed_xats.is_some() {
        instance.xats_identity_key = Some(uuid::Uuid::new_v4().to_string());
    }
    Ok(instance)
}

pub(super) fn mutate(
    storage: &Storage,
    instances: &mut Vec<Instance>,
    groups: &[Group],
    index: usize,
    command: ManagedCommand,
) -> Result<Value> {
    ensure_unique_name(instances, &instances[index])?;
    let instance = &mut instances[index];
    let outcome = match command {
        ManagedCommand::Start(_) => instance.start_with_size(crate::terminal::get_size()),
        ManagedCommand::Restart(_) => instance.restart_with_size(crate::terminal::get_size()),
        ManagedCommand::Stop(_) => {
            instance.stop()?;
            instance.status = Status::Stopped;
            Ok(())
        }
        ManagedCommand::Rename(args) => rename(instance, &args.title),
        ManagedCommand::Send(args) => return send(instance, &args.message),
        ManagedCommand::Capture(args) => return capture(instance, args.lines),
        ManagedCommand::Remove(_) => return remove(storage, instances, groups, index),
        _ => unreachable!(),
    };
    let id = instance.id.clone();
    save(storage, instances, groups)?;
    outcome?;
    Ok(json!({"id": id, "status": instances[index].status,
        "xats": crate::managed_xats::status(&instances[index])?}))
}

fn validate_title(title: &str) -> Result<()> {
    ensure!(
        !title.trim().is_empty() && title.len() <= 200 && !title.chars().any(char::is_control),
        "Invalid session title"
    );
    Ok(())
}

fn rename(instance: &mut Instance, title: &str) -> Result<()> {
    validate_title(title)?;
    let session = crate::tmux::Session::new(&instance.id, &instance.title)?;
    if session.exists() {
        session.rename(&crate::tmux::Session::generate_name(
            &instance.id,
            title.trim(),
        ))?;
    }
    instance.title = title.trim().to_string();
    Ok(())
}

fn running_session(instance: &Instance) -> Result<crate::tmux::Session> {
    let session = crate::tmux::Session::new(&instance.id, &instance.title)?;
    ensure!(session.exists(), "Session is not running");
    Ok(session)
}

fn send(instance: &Instance, message: &str) -> Result<Value> {
    ensure!(
        !message.trim().is_empty() && message.len() <= 65536 && !message.contains('\0'),
        "Invalid message"
    );
    running_session(instance)?.send_keys(message)?;
    Ok(json!({"id": instance.id, "sent": true}))
}

fn capture(instance: &Instance, lines: u32) -> Result<Value> {
    running_session(instance)?;
    let name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
    let output = crate::tmux::tmux_command()
        .args([
            "capture-pane",
            "-t",
            &format!("={name}:^.0"),
            "-p",
            "-e",
            "-S",
            &format!("-{lines}"),
        ])
        .output()
        .context("Capturing session output")?;
    ensure!(
        output.status.success(),
        "Capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let content = String::from_utf8_lossy(&output.stdout);
    Ok(json!({"id": instance.id, "content": content}))
}

fn remove(
    storage: &Storage,
    instances: &mut Vec<Instance>,
    groups: &[Group],
    index: usize,
) -> Result<Value> {
    persist_release_preparation(storage, instances, groups, index)?;
    let instance = &instances[index];
    let id = instance.id.clone();
    instance.stop()?;
    crate::managed_xats::release(instance)?;
    if instance.tool == "codex" {
        crate::managed_codex::cleanup(instance)?;
    }
    crate::managed_runtimes::cleanup(instance)?;
    let store = crate::db::Store::open_with_schema(storage.profile())?;
    store.delete_slots_for_instance(&id)?;
    instances.remove(index);
    save(storage, instances, groups)?;
    Ok(json!({"id": id, "removed": true, "code_resources_preserved": true}))
}

fn persist_release_preparation(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    index: usize,
) -> Result<()> {
    crate::managed_xats::prepare_release(&mut instances[index])?;
    save(storage, instances, groups)
}

fn replace(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    instance: &Instance,
) -> Result<()> {
    let target = instances
        .iter_mut()
        .find(|item| item.id == instance.id)
        .context("Created session is missing")?;
    *target = instance.clone();
    save(storage, instances, groups)
}

fn save(storage: &Storage, instances: &[Instance], groups: &[Group]) -> Result<()> {
    storage.save_with_groups(instances, &GroupTree::new_with_groups(instances, groups))
}

fn ensure_unique_name(instances: &[Instance], instance: &Instance) -> Result<()> {
    let name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
    ensure!(
        !instances.iter().any(|other| other.id != instance.id
            && crate::tmux::Session::generate_name(&other.id, &other.title) == name),
        "Session tmux name collides with another record"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn release_preparation_persists_recovered_fence_and_updates_save_baseline() {
        let directory = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(directory.path());
        let storage = Storage::new("release-fixture").unwrap();
        let mut instance = Instance::new("worker", "/tmp");
        instance.managed_xats = Some(
            serde_json::from_value(json!({
                "name": "worker", "team": "test",
                "launch_instance_id": "00000000-0000-4000-8000-000000000001",
                "claim": {"fence": {
                    "claim_id": "00000000-0000-4000-8000-000000000002",
                    "instance_id": "00000000-0000-4000-8000-000000000001",
                    "generation": 1
                }, "mcp_token": "private", "runtime": null}
            }))
            .unwrap(),
        );
        let mut instances = vec![instance];
        save(&storage, &instances, &[]).unwrap();
        let recovered = instances[0].managed_xats.as_mut().unwrap();
        recovered.launch_instance_id = "00000000-0000-4000-8000-000000000003".to_string();
        let claim = recovered.claim.as_mut().unwrap();
        claim.fence.instance_id = recovered.launch_instance_id.clone();
        claim.fence.generation = 2;
        persist_release_preparation(&storage, &mut instances, &[], 0).unwrap();
        let durable = Storage::new("release-fixture").unwrap().load().unwrap();
        assert_eq!(durable[0].managed_xats, instances[0].managed_xats);
        instances.clear();
        save(&storage, &instances, &[]).unwrap();
        assert!(Storage::new("release-fixture")
            .unwrap()
            .load()
            .unwrap()
            .is_empty());
    }
}
