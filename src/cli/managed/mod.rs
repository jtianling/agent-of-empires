mod actions;
pub mod args;
mod context;
pub mod pane_args;
mod panes;

use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};

use crate::session::{GroupTree, Instance, Storage};
use args::{ManagedArgs, ManagedCommand, SetOwnerArgs};

#[derive(Debug, thiserror::Error)]
#[error("{code}")]
struct Failure {
    code: &'static str,
    session_id: Option<String>,
    pane_id: Option<String>,
}

fn coded(error: anyhow::Error, code: &'static str, id: Option<&str>) -> anyhow::Error {
    error.context(Failure {
        code,
        session_id: id.map(str::to_owned),
        pane_id: None,
    })
}

fn coded_pane(
    error: anyhow::Error,
    code: &'static str,
    session: &str,
    pane: &str,
) -> anyhow::Error {
    error.context(Failure {
        code,
        session_id: Some(session.into()),
        pane_id: Some(pane.into()),
    })
}

pub fn run(profile: &str, args: ManagedArgs) -> Result<()> {
    report(execute(profile, args))
}

fn report(result: Result<Value>) -> Result<()> {
    match result {
        Ok(data) => {
            println!("{}", json!({"ok": true, "data": data}));
            Ok(())
        }
        Err(error) => {
            let failure = error.downcast_ref::<Failure>();
            let code = failure.map_or("operation_failed", |failure| failure.code);
            let id = failure.and_then(|failure| failure.session_id.as_deref());
            let pane_id = failure.and_then(|failure| failure.pane_id.as_deref());
            let message = format!("{error:#}");
            tracing::error!(code, session_id = id, "Managed CLI failed: {message}");
            eprintln!("{message}");
            println!(
                "{}",
                json!({"ok": false, "error": {
                    "code": code, "message": message, "session_id": id, "pane_id": pane_id
                }})
            );
            bail!("Managed operation failed ({code})")
        }
    }
}

fn execute(profile: &str, args: ManagedArgs) -> Result<Value> {
    validate_profile(profile).map_err(|e| coded(e, "invalid_input", None))?;
    let path = args
        .context
        .as_deref()
        .context("Specify --context or AOE_MANAGER_CONTEXT")
        .map_err(|e| coded(e, "invalid_context", None))?;
    if matches!(args.command, ManagedCommand::Init) {
        let owner = context::initialize(path).map_err(|e| coded(e, "context_init_failed", None))?;
        return Ok(json!({"owner": owner, "context": path}));
    }
    let owner = context::authenticate(path).map_err(|e| coded(e, "invalid_context", None))?;
    let storage = Storage::new(profile)?;
    let _guard = storage.operation_lock()?;
    let (mut instances, groups) = storage.load_with_groups()?;
    for instance in &mut instances {
        instance.source_profile = profile.to_string();
    }
    match args.command {
        ManagedCommand::Init => unreachable!(),
        ManagedCommand::Create(args) => actions::create(&storage, instances, groups, &owner, args),
        ManagedCommand::Pane { command } => {
            panes::run(&storage, &mut instances, &groups, &owner, command)
        }
        ManagedCommand::List => Ok(json!(instances
            .iter()
            .map(|instance| metadata(instance, &owner))
            .collect::<Vec<_>>())),
        ManagedCommand::Show(args) => {
            let index = exact_index(&instances, &args.id)?;
            let mut output = metadata(&instances[index], &owner);
            if instances[index].managed_owner.as_deref() == Some(&owner) {
                output["xats"] = crate::managed_xats::status_report(&instances[index]);
            }
            Ok(output)
        }
        command => {
            let id = actions::target_id(&command).to_string();
            let index = owned_index(&instances, &id, &owner)?;
            let result = actions::mutate(&storage, &mut instances, &groups, index, command);
            result.map_err(|error| coded(error, "operation_failed", Some(&id)))
        }
    }
}

pub fn set_owner(profile: &str, args: SetOwnerArgs) -> Result<()> {
    report((|| {
        validate_profile(profile)?;
        let owner = args
            .context
            .as_deref()
            .map(context::authenticate)
            .transpose()?;
        let storage = Storage::new(profile)?;
        let _guard = storage.operation_lock()?;
        let (mut instances, groups) = storage.load_with_groups()?;
        let index = exact_index(&instances, &args.id)?;
        let previous = instances[index].managed_owner.clone();
        for pane in &mut instances[index].managed_panes {
            if pane.owner == previous && previous.is_some() {
                pane.owner = owner.clone();
            }
        }
        instances[index].managed_owner = owner;
        storage.save_with_groups(&instances, &GroupTree::new_with_groups(&instances, &groups))?;
        Ok(json!({"id": args.id, "owner": instances[index].managed_owner}))
    })())
}

fn validate_profile(profile: &str) -> Result<()> {
    ensure!(
        !profile.is_empty()
            && profile.len() <= 128
            && profile
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "Invalid profile name"
    );
    Ok(())
}

fn exact_index(instances: &[Instance], id: &str) -> Result<usize> {
    ensure!(
        id.len() == 16 && id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        coded(
            anyhow::anyhow!("Use the complete 16-character session ID"),
            "invalid_id",
            Some(id)
        )
    );
    instances
        .iter()
        .position(|instance| instance.id == id)
        .ok_or_else(|| coded(anyhow::anyhow!("Session not found"), "not_found", Some(id)))
}

fn owned_index(instances: &[Instance], id: &str, owner: &str) -> Result<usize> {
    let index = exact_index(instances, id)?;
    ensure!(
        instances[index].managed_owner.as_deref() == Some(owner),
        coded(
            anyhow::anyhow!("Session is not owned by this manager"),
            "permission_denied",
            Some(id)
        )
    );
    Ok(index)
}

fn metadata(instance: &Instance, owner: &str) -> Value {
    json!({"id": instance.id, "title": instance.title, "path": instance.project_path,
        "group": instance.group_path, "tool": instance.tool,
        "owner": instance.managed_owner,
        "owned": instance.managed_owner.as_deref() == Some(owner),
        "status": format!("{:?}", instance.status).to_lowercase(),
        "status_source": "stored", "created_at": instance.created_at})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_rejects_human_and_other_manager() {
        let mut instance = Instance::new("human", "/tmp");
        let id = instance.id.clone();
        assert!(owned_index(std::slice::from_ref(&instance), &id, "manager").is_err());
        instance.managed_owner = Some("other".into());
        assert!(owned_index(std::slice::from_ref(&instance), &id, "manager").is_err());
        instance.managed_owner = Some("manager".into());
        assert_eq!(owned_index(&[instance], &id, "manager").unwrap(), 0);
    }

    #[test]
    fn target_rejects_title_and_prefix() {
        let instance = Instance::new("title", "/tmp");
        let id = instance.id.clone();
        assert!(exact_index(std::slice::from_ref(&instance), "title").is_err());
        assert!(exact_index(std::slice::from_ref(&instance), &id[..8]).is_err());
        assert!(exact_index(&[instance], "").is_err());
    }

    #[test]
    fn metadata_never_contains_runtime_secrets() {
        let mut instance = Instance::new("example", "/tmp");
        instance.command = "private-command".into();
        instance.xats_identity_key = Some("private-identity".into());
        let output = metadata(&instance, "other").to_string();
        assert!(!output.contains("private-command"));
        assert!(!output.contains("private-identity"));
    }
}
