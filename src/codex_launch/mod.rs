mod arguments;
mod command;
mod config;
mod rpc;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use clap::Args;
use serde_json::json;

use crate::db::{AgentSlot, Store};
use crate::session::{Instance, Storage};
use command::{Command, Intent};

#[derive(Args)]
pub struct CodexLaunchArgs {
    #[arg(long)]
    instance_id: String,
    #[arg(long)]
    launch_id: uuid::Uuid,
    #[arg(long)]
    endpoint: String,
    #[arg(long)]
    working_directory: PathBuf,
    #[arg(long)]
    command_file: PathBuf,
    #[arg(skip)]
    command: Vec<String>,
}

pub fn write_arguments() -> Result<()> {
    let path = arguments::write()?;
    println!("{}", path.to_str().context("Invalid Codex argument path")?);
    Ok(())
}

#[derive(Default)]
struct FailureContext {
    slot: Option<AgentSlot>,
    secrets: Vec<String>,
}

pub async fn run(profile: &str, mut args: CodexLaunchArgs) -> Result<()> {
    crate::session::tui_instances::validate_profile(profile)?;
    crate::session::managed_pane::validate_pane_id(&args.instance_id)?;
    let mut context = FailureContext::default();
    let result = launch(profile, &mut args, &mut context).await;
    if let Err(error) = &result {
        let message = redact(format!("{error:#}"), &args, context.secrets);
        if let Err(log_error) = record_failure(profile, &args, &message, context.slot.as_ref()) {
            return Err(anyhow::anyhow!(
                "{message}; Recording launch failure failed: {log_error:#}"
            ));
        }
        return Err(anyhow::anyhow!(message));
    }
    result
}

async fn launch(
    profile: &str,
    args: &mut CodexLaunchArgs,
    context: &mut FailureContext,
) -> Result<()> {
    args.command = arguments::read(&args.command_file)?;
    let (cwd, pane, key) = launcher_identity(args)?;
    wait_for_slot(profile, &args.instance_id, &pane, &key).await?;
    let storage = Storage::new(profile)?;
    let _operation = storage.operation_lock()?;
    let store = Store::open_with_schema(profile)?;
    let (instance, slot, command) =
        launch_inputs(&storage, &store, args, &cwd, &pane, &key, context)?;
    context.secrets.extend(command.secrets());
    let thread = tokio::time::timeout(
        Duration::from_secs(30),
        prepare(&store, &slot, &instance, args, &command, &cwd, context),
    )
    .await
    .context("Timed out preparing the exact Codex launch thread")??;
    execute(&store, &slot, &instance, &pane, &key, &command, &thread)
}

fn launcher_identity(args: &CodexLaunchArgs) -> Result<(PathBuf, String, String)> {
    let endpoint = url::Url::parse(&args.endpoint).context("Invalid Codex launcher endpoint")?;
    ensure!(
        matches!(endpoint.scheme(), "ws" | "wss") && endpoint.host_str().is_some(),
        "Codex launcher requires a WebSocket endpoint"
    );
    let cwd = args
        .working_directory
        .canonicalize()
        .context("Resolving Codex launch directory")?;
    let pane = std::env::var("TMUX_PANE").context("Codex launcher requires its own pane")?;
    ensure!(
        crate::cli::record_pane::pane_hosts_this_process(&pane) == Some(true),
        "Codex launcher is not hosted by its requested pane"
    );
    let key = std::env::var(crate::xats_control::IDENTITY_KEY_ENV)
        .context("Codex launcher requires its pane identity key")?;
    crate::xats_control::validate_identity_key(&key)?;
    Ok((cwd, pane, key))
}

fn launch_inputs(
    storage: &Storage,
    store: &Store,
    args: &CodexLaunchArgs,
    cwd: &std::path::Path,
    pane: &str,
    key: &str,
    context: &mut FailureContext,
) -> Result<(Instance, AgentSlot, Command)> {
    let slot = own_slot(store, &args.instance_id, pane, key)?
        .context("Codex launcher slot changed before preparation")?;
    ensure!(
        slot.agent == "codex" && slot.cross_agent_team,
        "Codex launcher slot tool changed"
    );
    ensure!(
        std::fs::canonicalize(&slot.cwd)? == cwd,
        "Codex slot directory changed"
    );
    let instance = storage
        .load()?
        .into_iter()
        .find(|item| item.id == args.instance_id)
        .context("Codex launcher session disappeared")?;
    ensure!(
        !crate::session::managed_pane::has_managed_state(&instance),
        "Managed Codex must use its owner-controlled launcher"
    );
    let location =
        crate::tmux::managed_pane::locate(pane)?.context("Codex launch pane disappeared")?;
    ensure!(
        location.session == crate::tmux::Session::generate_name(&instance.id, &instance.title),
        "Codex launch pane belongs to a different session"
    );
    context.slot = Some(slot.clone());
    let command = Command::parse(&args.command, &args.endpoint, cwd)?;
    verify_launch_identity(&command, args.launch_id)?;
    Ok((instance, slot, command))
}

fn verify_launch_identity(command: &Command, launch: uuid::Uuid) -> Result<()> {
    for field in [
        "xats.agent_id",
        "shell_environment_policy.set.XATS_CODEX_LAUNCH_ID",
    ] {
        ensure!(
            command
                .config
                .get(field)
                .and_then(|value| value.as_str())
                .is_some_and(|id| uuid::Uuid::parse_str(id).ok() == Some(launch)),
            "Codex launch identity differs from its command"
        );
    }
    Ok(())
}

async fn wait_for_slot(profile: &str, instance: &str, pane: &str, key: &str) -> Result<()> {
    let store = Store::open_with_schema(profile)?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if own_slot(&store, instance, pane, key)?.is_some() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("Codex slot metadata was not recorded before launch")?
}

fn own_slot(store: &Store, instance: &str, pane: &str, key: &str) -> Result<Option<AgentSlot>> {
    let slots = store.read_slots_for_instance(instance)?;
    let matches = slots
        .into_iter()
        .filter(|slot| slot.tmux_pane == pane && slot.xats_identity_key == key)
        .collect::<Vec<_>>();
    ensure!(
        matches.len() <= 1,
        "Codex pane has ambiguous slot ownership"
    );
    Ok(matches.into_iter().next())
}

async fn prepare(
    store: &Store,
    slot: &AgentSlot,
    instance: &Instance,
    args: &CodexLaunchArgs,
    command: &Command,
    cwd: &std::path::Path,
    context: &mut FailureContext,
) -> Result<String> {
    if let Some(thread) = original_thread(store, slot, args, command, cwd)? {
        return Ok(thread);
    }
    let mut socket = rpc::connect(&args.endpoint).await?;
    let (thread, provider, response) = rpc::create(&mut socket, command, cwd).await?;
    store.persist_launched_codex_thread(slot, &thread, &args.launch_id.to_string())?;
    context.slot = Some(AgentSlot {
        native_session_id: thread.clone(),
        xats_runtime_generation: slot.xats_runtime_generation + 1,
        ..slot.clone()
    });
    store.save_runtime_settings(
        context
            .slot
            .as_ref()
            .context("Missing Codex creation receipt")?,
        &rpc::settings(&response)?,
    )?;
    rpc::finish(
        &mut socket,
        command,
        &thread,
        &provider,
        response,
        &instance.title,
    )
    .await?;
    Ok(thread)
}

fn original_thread(
    store: &Store,
    slot: &AgentSlot,
    args: &CodexLaunchArgs,
    command: &Command,
    cwd: &std::path::Path,
) -> Result<Option<String>> {
    if let Intent::Resume(thread) = &command.intent {
        ensure!(
            slot.native_session_id == *thread && store.codex_resume_verified(slot)?,
            "Codex resume has no durable proof for its exact original UUID"
        );
        ensure!(
            command.approval.is_none(),
            "Remote resume cannot override CLI permissions"
        );
        crate::codex_permissions::preflight_resume(
            &args.endpoint,
            thread,
            cwd.to_str().context("Invalid Codex working directory")?,
        )?;
        return Ok(Some(thread.clone()));
    }
    Ok(None)
}

fn execute(
    store: &Store,
    slot: &AgentSlot,
    instance: &Instance,
    pane: &str,
    key: &str,
    command: &Command,
    thread: &str,
) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let current =
        own_slot(store, &instance.id, pane, key)?.context("Codex slot moved before UI launch")?;
    let generation =
        slot.xats_runtime_generation + i64::from(!matches!(command.intent, Intent::Resume(_)));
    ensure!(
        current.xats_runtime_generation == generation && current.native_session_id == thread,
        "Codex thread changed before UI launch"
    );
    ensure!(
        current.agent == slot.agent
            && current.cwd == slot.cwd
            && current.yolo_mode == slot.yolo_mode
            && current.cross_agent_team == slot.cross_agent_team
            && current.worktree_info == slot.worktree_info,
        "Codex launch configuration changed before UI launch"
    );
    let location = crate::tmux::managed_pane::locate(pane)?.context("Codex pane disappeared")?;
    ensure!(
        location.session == crate::tmux::Session::generate_name(&instance.id, &instance.title),
        "Codex pane moved before UI launch"
    );
    ensure!(
        crate::cli::record_pane::pane_hosts_this_process(pane) == Some(true),
        "Codex pane changed before UI launch"
    );
    let error = std::process::Command::new(&command.program)
        .args(command.resume_arguments(thread))
        .exec();
    Err(error).context("Executing Codex UI")
}

fn redact(mut message: String, args: &CodexLaunchArgs, extra: Vec<String>) -> String {
    let mut secrets = [
        crate::xats_control::IDENTITY_KEY_ENV,
        "CROSS_AGENT_TEAMS_MCP_TOKEN",
    ]
    .into_iter()
    .filter_map(|name| std::env::var(name).ok())
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>();
    secrets.extend(command::private_values(&args.command));
    secrets.extend(extra);
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for secret in secrets {
        message = message.replace(&secret, "***");
    }
    message
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(16_384)
        .collect()
}

fn record_failure(
    profile: &str,
    args: &CodexLaunchArgs,
    message: &str,
    expected: Option<&AgentSlot>,
) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = crate::session::get_app_dir_path()?.join("debug.log");
    let mut log = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    writeln!(
        log,
        "{} ERROR Codex launcher {}: {}",
        chrono::Utc::now().to_rfc3339(),
        args.instance_id,
        message
    )?;
    let Some(expected) = expected else {
        return Ok(());
    };
    let pane = &expected.tmux_pane;
    let key = &expected.xats_identity_key;
    let store = Store::open_with_schema(profile)?;
    if let Some(slot) = own_slot(&store, &args.instance_id, pane, key)? {
        if slot.xats_runtime_generation != expected.xats_runtime_generation
            || slot.native_session_id != expected.native_session_id
            || slot.agent != expected.agent
        {
            return Ok(());
        }
        let detail = json!({"pane":pane,"generation":slot.xats_runtime_generation,
            "thread":slot.native_session_id,"error":message})
        .to_string();
        store.append_event(
            &args.instance_id,
            Some(slot.slot),
            "codex_launch_failed",
            Some(&detail),
            crate::db::now_unix(),
        )?;
    }
    Ok(())
}
