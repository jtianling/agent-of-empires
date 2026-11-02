use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout, Command};

use super::{current_provider, RpcError, TIMEOUT};

struct ReaderGroup(Option<Pid>);

impl ReaderGroup {
    fn stop(&mut self) -> Result<()> {
        let Some(pid) = self.0 else {
            return Ok(());
        };
        match killpg(pid, Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => {
                self.0 = None;
                Ok(())
            }
            Err(error) => Err(error).context("stopping Codex reader process group"),
        }
    }
}

impl Drop for ReaderGroup {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            tracing::error!("Cannot clean up Codex configuration reader: {error:#}");
        }
    }
}

pub(crate) fn current_local_provider(working_dir: &str) -> Result<String> {
    current_provider(current_local_config(working_dir, false)?)
}

pub(crate) fn current_local_config(working_dir: &str, trust_working_dir: bool) -> Result<Value> {
    local_config(working_dir, trust_working_dir, None)
}

pub(crate) fn current_local_resume_provider(working_dir: &str, thread: &str) -> Result<String> {
    current_provider(local_config(working_dir, false, Some(thread.to_string()))?)
}

fn local_config(
    working_dir: &str,
    trust_working_dir: bool,
    thread: Option<String>,
) -> Result<Value> {
    let binary = crate::agents::executable::resolve("codex")
        .context("cannot find Codex to read the current model provider")?;
    let working_dir = working_dir.to_string();
    crate::xats_control::block_on_control("codex-config", move || {
        Box::pin(async move {
            read_config(&binary, &working_dir, trust_working_dir, thread.as_deref()).await
        })
    })
}

async fn read_config(
    binary: &Path,
    working_dir: &str,
    trust_working_dir: bool,
    thread: Option<&str>,
) -> Result<Value> {
    if thread.is_some() {
        start_daemon(binary, working_dir).await?;
    }
    let mut command = Command::new(binary);
    if trust_working_dir {
        command.args(["-c", &super::project_trust_override(working_dir)]);
    }
    command.args(if thread.is_some() {
        vec!["app-server", "proxy"]
    } else {
        vec!["app-server", "--listen", "stdio://"]
    });
    let mut child = command
        .current_dir(working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .spawn()
        .context("starting Codex configuration reader")?;
    let pid = child.id().context("Codex reader has no process ID")?;
    let mut group = ReaderGroup(Some(Pid::from_raw(i32::try_from(pid)?)));
    let stderr = child.stderr.take().context("Codex reader has no stderr")?;
    let errors = tokio::spawn(async move {
        let mut output = Vec::new();
        stderr.take(64 * 1024).read_to_end(&mut output).await?;
        Ok::<_, std::io::Error>(String::from_utf8_lossy(&output).into_owned())
    });
    let result = tokio::time::timeout(TIMEOUT, async {
        match thread {
            Some(thread) => read_resume_provider(&mut child, working_dir, thread).await,
            None => read_provider(&mut child, working_dir).await,
        }
    })
    .await
    .context("timed out reading current Codex model provider")
    .and_then(|result| result);
    stop_reader(&mut child, &mut group).await?;
    group.stop()?;
    let stderr = tokio::time::timeout(Duration::from_secs(2), errors)
        .await
        .context("timed out joining Codex configuration diagnostics")?
        .context("joining Codex configuration diagnostics")?
        .context("reading Codex configuration diagnostics")?;
    match result {
        Ok(provider) => Ok(provider),
        Err(error) if stderr.trim().is_empty() => Err(error),
        Err(error) => Err(error.context(format!("Codex configuration reader: {}", stderr.trim()))),
    }
}

async fn start_daemon(binary: &Path, working_dir: &str) -> Result<()> {
    let output = tokio::time::timeout(
        TIMEOUT,
        Command::new(binary)
            .args(["app-server", "daemon", "start"])
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CODEX_THREAD_ID")
            .env_remove("AOE_INSTANCE_ID")
            .env_remove(crate::xats_control::IDENTITY_KEY_ENV)
            .env_remove(crate::managed_runtimes::MANAGER_CONTEXT_ENV)
            .env_remove(crate::managed_runtimes::MCP_TOKEN_ENV)
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Timed out ensuring the local Codex daemon is running")?
    .context("Starting the local Codex daemon for resume")?;
    if !output.status.success() {
        bail!(
            "Cannot start local Codex daemon: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

async fn stop_reader(child: &mut tokio::process::Child, group: &mut ReaderGroup) -> Result<()> {
    match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(_)) => return Ok(()),
        Ok(Err(error)) => return Err(error).context("waiting for Codex configuration reader"),
        Err(_) => {}
    }
    group.stop()?;
    tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .context("timed out stopping Codex configuration reader")?
        .context("stopping Codex configuration reader")?;
    Ok(())
}

async fn read_provider(child: &mut tokio::process::Child, cwd: &str) -> Result<Value> {
    let mut input = child.stdin.take().context("Codex reader has no stdin")?;
    let mut output =
        BufReader::new(child.stdout.take().context("Codex reader has no stdout")?).lines();
    exchange(
        &mut input,
        &mut output,
        1,
        "initialize",
        json!({"clientInfo": {"name": "aoe", "version": env!("CARGO_PKG_VERSION")}}),
    )
    .await?;
    input
        .write_all(b"{\"method\":\"initialized\"}\n")
        .await
        .context("initializing Codex configuration reader")?;
    let config = exchange(
        &mut input,
        &mut output,
        2,
        "config/read",
        json!({"includeLayers": false, "cwd": cwd}),
    )
    .await?;
    Ok(config)
}

async fn read_resume_provider(
    child: &mut tokio::process::Child,
    cwd: &str,
    thread: &str,
) -> Result<Value> {
    let input = child.stdin.take().context("Codex proxy has no stdin")?;
    let output = child.stdout.take().context("Codex proxy has no stdout")?;
    let transport = tokio::io::join(output, input);
    let (mut socket, _) = tokio_tungstenite::client_async("ws://localhost/", transport)
        .await
        .context("Connecting to the local Codex daemon through its proxy")?;
    super::request(
        &mut socket,
        1,
        "initialize",
        json!({
            "clientInfo": {"name": "aoe", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }),
    )
    .await?;
    use futures_util::SinkExt;
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"method": "initialized"}).to_string().into(),
        ))
        .await?;
    let config = super::request(
        &mut socket,
        2,
        "config/read",
        json!({"includeLayers": false, "cwd": cwd}),
    )
    .await?;
    let provider = current_provider(config.clone())?;
    let response = super::resume_current_provider(
        &mut socket,
        3,
        json!({"threadId": thread, "cwd": cwd, "modelProvider": provider, "excludeTurns": true}),
        &config,
        true,
    )
    .await?;
    super::validate_resume(response, thread, &provider, false)?;
    socket
        .close(None)
        .await
        .context("Closing the local Codex resume connection")?;
    Ok(config)
}

async fn exchange(
    input: &mut ChildStdin,
    output: &mut Lines<BufReader<ChildStdout>>,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value> {
    let line = format!(
        "{}\n",
        json!({"id": id, "method": method, "params": params})
    );
    input
        .write_all(line.as_bytes())
        .await
        .with_context(|| format!("sending local Codex {method}"))?;
    while let Some(line) = output
        .next_line()
        .await
        .with_context(|| format!("reading local Codex {method}"))?
    {
        let response: Value = serde_json::from_str(&line)
            .with_context(|| format!("invalid local Codex {method} response"))?;
        if response.get("id") != Some(&json!(id)) {
            continue;
        }
        if let Some(error) = response.get("error") {
            let error: RpcError = serde_json::from_value(error.clone())
                .context("invalid local Codex RPC error response")?;
            bail!(
                "Codex rejected {method} (code {}): {}",
                error.code,
                error.message
            );
        }
        return response
            .get("result")
            .cloned()
            .with_context(|| format!("local Codex {method} response has no result"));
    }
    bail!("Codex configuration reader disconnected during {method}")
}
