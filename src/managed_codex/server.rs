use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::session::Instance;

const TOKEN_ENV: &str = "AOE_MANAGED_CODEX_MCP_TOKEN";
const START_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_DELAY: Duration = Duration::from_millis(50);
const STOP_ATTEMPTS: usize = 40;
const LOG_LIMIT: u64 = 65536;

#[cfg(test)]
mod tests;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Server {
    pane_id: String,
    launch_id: String,
    pid: u32,
    start_identity: String,
    endpoint: Option<String>,
    #[serde(default)]
    members: Vec<(u32, String)>,
}

pub(super) fn start(
    instance: &Instance,
    prepared: &super::PreparedThread,
    mcp_url: &url::Url,
) -> Result<String> {
    stop(instance, false)?;
    let _lock = lock(instance)?;
    let log_path = record_path(instance)?.with_extension("stderr.log");
    super::write_private(&log_path, b"")?;
    let log = OpenOptions::new().append(true).open(&log_path)?;
    let binary = crate::agents::executable::resolve("codex").context("Codex is unavailable")?;
    let mut command = server_command(&binary, instance, prepared, mcp_url, log)?;
    let reaper = start_reaper()?;
    let mut child = command.spawn().context("Starting owned Codex app-server")?;
    let mut server = None;
    let result = record_and_wait(instance, &mut child, &log_path, &mut server).map_err(|error| {
        startup_error(
            instance,
            &mut child,
            server.as_ref(),
            &log_path,
            prepared,
            error,
        )
    });
    match reaper.send(child) {
        Ok(()) => result,
        Err(error) => {
            let mut child = error.0;
            let handoff = startup_error(
                instance,
                &mut child,
                server.as_ref(),
                &log_path,
                prepared,
                anyhow::anyhow!("Owned Codex reaper disconnected before handoff"),
            );
            let reaped = reap_failed_handoff(child);
            let error = result
                .err()
                .into_iter()
                .chain(Some(handoff))
                .chain(reaped.err())
                .reduce(|first, second| anyhow::anyhow!("{first:#}; {second:#}"))
                .context("Owned Codex reaper handoff failed")?;
            Err(error)
        }
    }
}

fn reap_failed_handoff(mut child: Child) -> Result<()> {
    if child.try_wait()?.is_none() {
        child
            .kill()
            .context("Stopping child after failed Codex reaper handoff")?;
    }
    child
        .wait()
        .context("Reaping child after failed Codex reaper handoff")?;
    Ok(())
}

fn start_reaper() -> Result<std::sync::mpsc::Sender<Child>> {
    let (sender, receiver) = std::sync::mpsc::channel::<Child>();
    std::thread::Builder::new()
        .name("aoe-codex-reaper".into())
        .spawn(move || match receiver.recv() {
            Ok(mut child) => {
                if let Err(error) = child.wait() {
                    tracing::error!("Failed to reap owned Codex app-server: {error}");
                }
            }
            Err(_) => tracing::debug!("Owned Codex startup canceled before reaper handoff"),
        })
        .context("Starting owned Codex child reaper")?;
    Ok(sender)
}

fn server_command(
    binary: &std::path::Path,
    instance: &Instance,
    prepared: &super::PreparedThread,
    mcp_url: &url::Url,
    log: File,
) -> Result<Command> {
    let token = crate::managed_xats::mcp_token(instance)?;
    let server_name = &prepared.server_name;
    let header_key = format!(
        "mcp_servers.{server_name}.env_http_headers.{}",
        super::HEADER
    );
    let url_key = format!("mcp_servers.{server_name}.url");
    let mut command = Command::new(binary);
    command
        .args(["-c", &format!("{header_key}=\"{TOKEN_ENV}\"")])
        .args([
            "-c",
            &format!("{url_key}={}", toml_string(mcp_url.as_str())?),
        ])
        .args([
            "-c",
            &format!("model_provider={}", toml_string(&prepared.provider)?),
        ])
        .args(["app-server", "--listen", "ws://127.0.0.1:0"])
        .current_dir(&instance.project_path)
        .env(TOKEN_ENV, token)
        .env("NO_COLOR", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove(crate::xats_control::IDENTITY_KEY_ENV)
        .env_remove(crate::managed_runtimes::MANAGER_CONTEXT_ENV)
        .env_remove(crate::managed_runtimes::MCP_TOKEN_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    if prepared.yolo {
        command.args([
            "-c",
            &crate::codex_permissions::project_trust_override(&instance.project_path),
        ]);
        command.args([
            "-c",
            "approval_policy=\"never\"",
            "-c",
            "sandbox_mode=\"danger-full-access\"",
        ]);
    }
    crate::process::isolate_process_group(&mut command);
    Ok(command)
}

fn startup_error(
    instance: &Instance,
    child: &mut Child,
    record: Option<&Server>,
    log_path: &std::path::Path,
    prepared: &super::PreparedThread,
    error: anyhow::Error,
) -> anyhow::Error {
    let detail = match read_log(log_path) {
        Ok(detail) => redact(detail, prepared),
        Err(error) => format!("Cannot read app-server diagnostics: {error:#}"),
    };
    let stopped = cleanup_child(instance, child, record);
    let reaped = match child.try_wait().context("Reaping failed owned Codex child") {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(anyhow::anyhow!(
            "Owned Codex child has not exited before reaper handoff"
        )),
        Err(error) => Err(error),
    };
    let removed = if stopped.is_ok() {
        remove_record(instance)
    } else {
        Ok(())
    };
    let errors = [Some(error), stopped.err(), reaped.err(), removed.err()]
        .into_iter()
        .flatten()
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>();
    anyhow::anyhow!("{}; app-server diagnostics: {detail}", errors.join("; "))
}

fn cleanup_child(instance: &Instance, child: &mut Child, record: Option<&Server>) -> Result<()> {
    let mut witnesses = record
        .map(|server| server.members.clone())
        .unwrap_or_default();
    if let Some(server) = record {
        witnesses.push((server.pid, server.start_identity.clone()));
    }
    if record.is_none() && child.try_wait()?.is_none() {
        if let Some(identity) = crate::process::start_identity(child.id())? {
            witnesses.push((child.id(), identity));
        }
    }
    stop_group(instance, child.id(), witnesses, record.cloned())?;
    Ok(())
}

fn remove_record(instance: &Instance) -> Result<()> {
    match fs::remove_file(record_path(instance)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Removing owned Codex server record"),
    }
}

fn lock(instance: &Instance) -> Result<File> {
    let path = record_path(instance)?.with_extension("lock");
    fs::create_dir_all(path.parent().context("Owned Codex lock has no parent")?)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    fs2::FileExt::lock_exclusive(&lock).context("Locking owned Codex server state")?;
    Ok(lock)
}

fn record_and_wait(
    instance: &Instance,
    child: &mut Child,
    log: &std::path::Path,
    state: &mut Option<Server>,
) -> Result<String> {
    let server = state.insert(Server {
        pane_id: instance.primary_pane_id().into(),
        launch_id: launch_id(instance)?,
        pid: child.id(),
        start_identity: crate::process::start_identity(child.id())?
            .context("Owned Codex process exited before registration")?,
        endpoint: None,
        members: Vec::new(),
    });
    write_record(instance, server)?;
    let started = Instant::now();
    while started.elapsed() < START_TIMEOUT {
        if let Some(status) = child.try_wait().context("Checking owned Codex startup")? {
            bail!("Owned Codex app-server exited during startup: {status}");
        }
        let members = crate::process::group_members(server.pid)?;
        if members != server.members {
            server.members = members;
            write_record(instance, server)?;
        }
        for line in read_log(log)?.lines() {
            if let Some(endpoint) = line.trim().strip_prefix("listening on: ") {
                let endpoint = url::Url::parse(endpoint).context("Invalid owned Codex endpoint")?;
                super::validate_endpoint(endpoint.as_str())?;
                ensure!(
                    endpoint.port().is_some_and(|port| port != 0),
                    "Invalid owned Codex port"
                );
                server.endpoint = Some(endpoint.to_string());
                server.members = crate::process::group_members(server.pid)?;
                write_record(instance, server)?;
                return Ok(endpoint.to_string());
            }
        }
        std::thread::sleep(POLL_DELAY);
    }
    bail!("Timed out waiting for owned Codex listener startup");
}

fn toml_string(value: &str) -> Result<String> {
    Ok(toml::Value::String(value.into()).to_string())
}

fn read_log(path: &std::path::Path) -> Result<String> {
    let mut text = String::new();
    File::open(path)?
        .take(LOG_LIMIT)
        .read_to_string(&mut text)?;
    Ok(text)
}

fn redact(mut text: String, prepared: &super::PreparedThread) -> String {
    if let Some(headers) = prepared.headers.as_object() {
        for value in headers.values().filter_map(serde_json::Value::as_str) {
            if !value.is_empty() {
                text = text.replace(value, "[redacted]");
            }
        }
    }
    text
}

fn record_path(instance: &Instance) -> Result<PathBuf> {
    crate::session::managed_pane::validate_pane_id(instance.primary_pane_id())?;
    let profile = instance.managed_profile();
    Ok(crate::session::get_profile_dir(&profile)?
        .join("managed-codex")
        .join(format!("{}.server.json", instance.primary_pane_id())))
}

fn launch_id(instance: &Instance) -> Result<String> {
    let identity = instance
        .managed_xats
        .as_ref()
        .context("Missing managed identity")?;
    let launch = identity
        .claim
        .as_ref()
        .context("Missing managed claim")?
        .fence
        .instance_id
        .clone();
    uuid::Uuid::try_parse(&launch).context("Invalid managed launch ID")?;
    Ok(launch)
}

fn write_record(instance: &Instance, server: &Server) -> Result<()> {
    super::write_private(&record_path(instance)?, &serde_json::to_vec(server)?)
}

fn read_record(instance: &Instance) -> Result<Option<Server>> {
    let path = record_path(instance)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Inspecting owned Codex record"),
    };
    ensure!(
        metadata.is_file() && metadata.len() <= LOG_LIMIT,
        "Invalid owned Codex record file"
    );
    let server: Server =
        serde_json::from_slice(&fs::read(path)?).context("Invalid owned Codex record")?;
    ensure!(
        server.pane_id == instance.primary_pane_id(),
        "Owned Codex pane identity mismatch"
    );
    ensure!(
        !crate::process::is_unsafe_kill_root(server.pid),
        "Invalid owned Codex process ID"
    );
    ensure!(
        !server.start_identity.is_empty(),
        "Missing owned Codex process identity"
    );
    uuid::Uuid::try_parse(&server.launch_id).context("Invalid owned Codex launch ID")?;
    if let Some(endpoint) = &server.endpoint {
        super::validate_endpoint(endpoint)?;
    }
    Ok(Some(server))
}

pub(super) fn verify(instance: &Instance, endpoint: &str) -> Result<()> {
    let _lock = lock(instance)?;
    let server = read_record(instance)?.context("Owned Codex server record is missing")?;
    ensure!(
        server.launch_id == launch_id(instance)?,
        "Owned Codex launch fence changed"
    );
    ensure!(
        server.endpoint.as_deref() == Some(endpoint),
        "Owned Codex endpoint changed"
    );
    ensure!(
        crate::process::start_identity(server.pid)?.as_deref() == Some(&server.start_identity),
        "Owned Codex process identity changed"
    );
    Ok(())
}

pub(super) fn stop(instance: &Instance, current_launch_only: bool) -> Result<()> {
    let _lock = lock(instance)?;
    let Some(server) = read_record(instance)? else {
        return Ok(());
    };
    if current_launch_only && server.launch_id != launch_id(instance)? {
        return Ok(());
    }
    if let Some(current) = crate::process::start_identity(server.pid)? {
        ensure!(
            current == server.start_identity,
            "Owned process start identity changed"
        );
    }
    let mut witnesses = server.members.clone();
    witnesses.push((server.pid, server.start_identity.clone()));
    stop_group(instance, server.pid, witnesses, Some(server))?;
    remove_record(instance)
}

fn stop_group(
    instance: &Instance,
    pid: u32,
    witnesses: Vec<(u32, String)>,
    record: Option<Server>,
) -> Result<()> {
    let mut persistence_errors = Vec::new();
    let stopped = stop_observed_group(instance, pid, witnesses, record, &mut persistence_errors);
    let completed = stopped.is_ok();
    let error = stopped
        .err()
        .into_iter()
        .chain(persistence_errors)
        .reduce(|first, second| anyhow::anyhow!("{first:#}; {second:#}"));
    match error {
        Some(error) if completed => {
            Err(error.context("Owned group stopped, but witness persistence failed"))
        }
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn stop_observed_group(
    instance: &Instance,
    pid: u32,
    mut witnesses: Vec<(u32, String)>,
    mut record: Option<Server>,
    persistence_errors: &mut Vec<anyhow::Error>,
) -> Result<()> {
    let members = crate::process::group_members(pid)?;
    if !members.is_empty() {
        ensure!(
            members.iter().any(|member| witnesses.contains(member)),
            "Owned Codex process group identity changed"
        );
        witnesses = members;
        if let Err(error) = store_witnesses(instance, &mut record, &witnesses) {
            persistence_errors.push(error);
        }
        crate::process::signal_verified_group(pid, &witnesses, false)?;
    }
    for attempt in 0..STOP_ATTEMPTS {
        match crate::process::group_members(pid)? {
            remaining if remaining.is_empty() => {
                return Ok(());
            }
            remaining => {
                ensure!(
                    remaining.iter().any(|member| witnesses.contains(member)),
                    "Owned Codex process group was reused"
                );
                if remaining != witnesses {
                    witnesses = remaining;
                    if let Err(error) = store_witnesses(instance, &mut record, &witnesses) {
                        persistence_errors.push(error);
                    }
                }
            }
        }
        if attempt == STOP_ATTEMPTS / 2 {
            crate::process::signal_verified_group(pid, &witnesses, true)?;
        }
        std::thread::sleep(POLL_DELAY);
    }
    bail!("Owned Codex app-server {pid} did not stop")
}

fn store_witnesses(
    instance: &Instance,
    record: &mut Option<Server>,
    members: &[(u32, String)],
) -> Result<()> {
    if let Some(record) = record {
        record.members = members.to_vec();
        write_record(instance, record).context("Saving owned process group witnesses")?;
    }
    Ok(())
}
