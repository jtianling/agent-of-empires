use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

const TIMEOUT: Duration = Duration::from_secs(15);
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Deserialize)]
struct ResumeResponse {
    thread: Thread,
    #[serde(rename = "approvalPolicy")]
    approval_policy: String,
    sandbox: Sandbox,
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
}

pub(crate) fn restore_yolo(endpoint: String, thread_id: String) -> Result<()> {
    uuid::Uuid::try_parse(&thread_id).context("invalid Codex resume thread ID")?;
    let url = url::Url::parse(&endpoint).context("invalid Codex app-server URL")?;
    if !matches!(url.scheme(), "ws" | "wss") || url.host_str().is_none() {
        bail!("Codex permissions require a ws:// or wss:// endpoint");
    }
    crate::xats_control::block_on_control("codex-permissions", move || {
        Box::pin(async move {
            tokio::time::timeout(TIMEOUT, restore(&endpoint, &thread_id))
                .await
                .context("timed out restoring Codex YOLO permissions")?
        })
    })
}

async fn restore(endpoint: &str, thread_id: &str) -> Result<()> {
    let (mut socket, _) = connect_async(endpoint)
        .await
        .context("connecting to Codex to restore YOLO permissions")?;
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
    let params = json!({"threadId": thread_id, "excludeTurns": true});
    let resumed = request(&mut socket, 2, "thread/resume", params.clone()).await?;
    validate_resume(resumed, thread_id, false)?;
    request(
        &mut socket,
        3,
        "thread/settings/update",
        json!({
            "threadId": thread_id,
            "approvalPolicy": "never",
            "sandboxPolicy": {"type": "dangerFullAccess"}
        }),
    )
    .await?;
    let verified = request(&mut socket, 4, "thread/resume", params).await?;
    validate_resume(verified, thread_id, true)?;
    socket
        .close(None)
        .await
        .context("closing Codex permissions client")
}

fn validate_resume(value: Value, thread_id: &str, require_yolo: bool) -> Result<()> {
    let response: ResumeResponse =
        serde_json::from_value(value).context("invalid Codex permissions response")?;
    if response.thread.id != thread_id {
        bail!("Codex permissions response belongs to a different thread");
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
                        "Codex rejected {method} while restoring YOLO permissions (code {})",
                        error.code
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
                    "approvalPolicy": approval,
                    "sandbox": {"type": sandbox}
                }),
                "target",
                true,
            )
            .is_err());
        }
    }

    #[test]
    fn validates_initial_identity_and_final_permissions() {
        let response = json!({
            "thread": {"id": "target"},
            "approvalPolicy": "never",
            "sandbox": {"type": "dangerFullAccess"}
        });
        assert!(validate_resume(response.clone(), "target", false).is_ok());
        assert!(validate_resume(response, "target", true).is_ok());
        assert!(validate_resume(json!({}), "target", true).is_err());
    }
}
