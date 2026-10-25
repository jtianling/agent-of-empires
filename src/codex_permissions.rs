use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

mod local;
pub(crate) use local::current_local_provider;

pub(crate) fn local_resume_command(cmd: &str, cwd: &str) -> Result<String> {
    use crate::session::environment::shell_escape;

    let executable = std::env::current_exe().context("locating AoE for Codex resume")?;
    let executable = executable
        .to_str()
        .context("AoE executable path is not UTF-8")?;
    let script = format!(
        "provider=$({} __codex-provider --cwd {}) || exit $?; \
         exec {cmd} --config \"model_provider=$provider\"",
        shell_escape(executable),
        shell_escape(cwd)
    );
    Ok(format!("sh -c {}", shell_escape(&script)))
}

const TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_MODEL_PROVIDER: &str = "openai";
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Deserialize)]
struct ResumeResponse {
    thread: Thread,
    #[serde(rename = "approvalPolicy")]
    approval_policy: String,
    sandbox: Sandbox,
    #[serde(rename = "modelProvider")]
    model_provider: String,
}

#[derive(Deserialize)]
struct ConfigResponse {
    config: ProviderConfig,
}

#[derive(Deserialize)]
struct ProviderConfig {
    model_provider: Option<String>,
}

#[derive(Deserialize)]
struct Thread {
    id: String,
}

#[derive(Deserialize)]
struct Sandbox {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

pub(crate) fn prepare_resume(
    endpoint: String,
    thread_id: String,
    working_dir: String,
    yolo: bool,
) -> Result<()> {
    uuid::Uuid::try_parse(&thread_id).context("invalid Codex resume thread ID")?;
    let url = url::Url::parse(&endpoint).context("invalid Codex app-server URL")?;
    if !matches!(url.scheme(), "ws" | "wss") || url.host_str().is_none() {
        bail!("Codex resume requires a ws:// or wss:// endpoint");
    }
    crate::xats_control::block_on_control("codex-permissions", move || {
        Box::pin(async move {
            tokio::time::timeout(TIMEOUT, restore(&endpoint, &thread_id, &working_dir, yolo))
                .await
                .context("timed out preparing Codex resume")?
        })
    })
}

async fn restore(endpoint: &str, thread_id: &str, working_dir: &str, yolo: bool) -> Result<()> {
    let (mut socket, _) = connect_async(endpoint)
        .await
        .context("connecting to Codex to prepare resume")?;
    request(
        &mut socket,
        1,
        "initialize",
        json!({
            "clientInfo": {"name": "aoe", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }),
    )
    .await?;
    socket
        .send(Message::Text(
            json!({"method": "initialized"}).to_string().into(),
        ))
        .await
        .context("initializing Codex permissions client")?;
    restore_thread(&mut socket, thread_id, working_dir, yolo).await?;
    socket
        .close(None)
        .await
        .context("closing Codex resume client")
}

async fn restore_thread(
    socket: &mut Socket,
    thread_id: &str,
    working_dir: &str,
    yolo: bool,
) -> Result<()> {
    let config = request(
        socket,
        2,
        "config/read",
        json!({"includeLayers": false, "cwd": working_dir}),
    )
    .await?;
    let provider = current_provider(config)?;
    let params = json!({
        "threadId": thread_id,
        "cwd": working_dir,
        "excludeTurns": true,
        "modelProvider": provider
    });
    let resumed = request(socket, 3, "thread/resume", params.clone()).await?;
    validate_resume(resumed, thread_id, &provider, false)?;
    if !yolo {
        return Ok(());
    }
    request(
        socket,
        4,
        "thread/settings/update",
        json!({
            "threadId": thread_id,
            "approvalPolicy": "never",
            "sandboxPolicy": {"type": "dangerFullAccess"}
        }),
    )
    .await?;
    let verified = request(socket, 5, "thread/resume", params).await?;
    validate_resume(verified, thread_id, &provider, true)
}

pub(super) fn current_provider(value: Value) -> Result<String> {
    let response: ConfigResponse =
        serde_json::from_value(value).context("invalid Codex config response")?;
    let provider = response
        .config
        .model_provider
        .unwrap_or_else(|| DEFAULT_MODEL_PROVIDER.to_string());
    if provider.trim().is_empty() {
        bail!("Codex current model provider is empty");
    }
    Ok(provider)
}

fn validate_resume(
    value: Value,
    thread_id: &str,
    provider: &str,
    require_yolo: bool,
) -> Result<()> {
    let response: ResumeResponse =
        serde_json::from_value(value).context("invalid Codex permissions response")?;
    if response.thread.id != thread_id {
        bail!("Codex permissions response belongs to a different thread");
    }
    if response.model_provider != provider {
        bail!("Codex did not retain the current model provider {provider}");
    }
    if require_yolo
        && (response.approval_policy != "never" || response.sandbox.kind != "dangerFullAccess")
    {
        bail!("Codex did not retain the requested YOLO permissions");
    }
    Ok(())
}

async fn request(socket: &mut Socket, id: u64, method: &str, params: Value) -> Result<Value> {
    socket
        .send(Message::Text(
            json!({"id": id, "method": method, "params": params})
                .to_string()
                .into(),
        ))
        .await
        .with_context(|| format!("sending Codex {method}"))?;
    while let Some(message) = socket.next().await {
        let message = message.with_context(|| format!("receiving Codex {method}"))?;
        match message {
            Message::Text(text) => {
                let response: Value = serde_json::from_str(&text)
                    .with_context(|| format!("invalid Codex {method} response"))?;
                if response.get("id") != Some(&json!(id)) {
                    continue;
                }
                if let Some(error) = response.get("error") {
                    let error: RpcError = serde_json::from_value(error.clone())
                        .context("invalid Codex RPC error response")?;
                    bail!(
                        "Codex rejected {method} while preparing resume \
                         (code {}): {}",
                        error.code,
                        error.message
                    );
                }
                return response
                    .get("result")
                    .cloned()
                    .with_context(|| format!("Codex {method} response has no result"));
            }
            Message::Ping(_) => socket.flush().await.context("replying to Codex ping")?,
            Message::Close(_) => break,
            _ => {}
        }
    }
    bail!("Codex disconnected during {method}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_thread_and_partial_yolo_permissions() {
        for (thread, approval, sandbox) in [
            ("other", "never", "dangerFullAccess"),
            ("target", "on-request", "dangerFullAccess"),
            ("target", "never", "workspaceWrite"),
        ] {
            assert!(validate_resume(
                json!({
                    "thread": {"id": thread},
                    "modelProvider": "current",
                    "approvalPolicy": approval,
                    "sandbox": {"type": sandbox}
                }),
                "target",
                "current",
                true,
            )
            .is_err());
        }
    }

    #[test]
    fn validates_initial_identity_and_final_permissions() {
        let response = json!({
            "thread": {"id": "target"},
            "modelProvider": "current",
            "approvalPolicy": "never",
            "sandbox": {"type": "dangerFullAccess"}
        });
        assert!(validate_resume(response.clone(), "target", "current", false).is_ok());
        assert!(validate_resume(response.clone(), "target", "old", false).is_err());
        assert!(validate_resume(response, "target", "current", true).is_ok());
        assert!(validate_resume(json!({}), "target", "current", true).is_err());
    }

    #[test]
    fn resolves_current_provider_and_rejects_invalid_config() {
        assert_eq!(
            current_provider(json!({"config": {"model_provider": "OpenAI"}})).unwrap(),
            "OpenAI"
        );
        assert_eq!(
            current_provider(json!({"config": {"model_provider": null}})).unwrap(),
            DEFAULT_MODEL_PROVIDER
        );
        for config in [json!({}), json!({"config": {"model_provider": " "}})] {
            assert!(current_provider(config).is_err());
        }
    }
}
