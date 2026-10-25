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
    let binary = crate::agents::executable::resolve("codex")
        .context("cannot find Codex to read the current model provider")?;
    let working_dir = working_dir.to_string();
    crate::xats_control::block_on_control("codex-config", move || {
        Box::pin(async move { read_config(&binary, &working_dir).await })
    })
}

async fn read_config(binary: &Path, working_dir: &str) -> Result<String> {
    let mut child = Command::new(binary)
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
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
    let result = tokio::time::timeout(TIMEOUT, read_provider(&mut child, working_dir))
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

async fn read_provider(child: &mut tokio::process::Child, cwd: &str) -> Result<String> {
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
    current_provider(config)
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
