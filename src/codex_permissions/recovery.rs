use anyhow::{ensure, Context, Result};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::{request, TIMEOUT};

#[derive(Deserialize)]
struct Response {
    thread: RecoveryThread,
}

#[derive(Deserialize)]
struct RecoveryThread {
    id: String,
    cwd: String,
    status: Status,
}

#[derive(Deserialize)]
struct Status {
    #[serde(rename = "type")]
    kind: String,
}

pub(crate) fn verify_recovery_thread(endpoint: &str, thread: &str, cwd: &str) -> Result<()> {
    uuid::Uuid::try_parse(thread).context("Invalid original Codex thread UUID")?;
    let url = url::Url::parse(endpoint).context("Invalid Codex recovery endpoint")?;
    ensure!(
        matches!(url.scheme(), "ws" | "wss") && url.host_str().is_some(),
        "Codex recovery requires a WebSocket endpoint"
    );
    let (endpoint, thread, cwd) = (endpoint.to_owned(), thread.to_owned(), cwd.to_owned());
    crate::xats_control::block_on_control("codex-recovery", move || {
        Box::pin(async move {
            tokio::time::timeout(TIMEOUT, inspect(&endpoint, &thread, &cwd))
                .await
                .context("Timed out verifying original Codex thread")?
        })
    })
}

async fn inspect(endpoint: &str, thread: &str, cwd: &str) -> Result<()> {
    let (mut socket, _) = connect_async(endpoint)
        .await
        .context("Connecting to Codex for explicit recovery")?;
    request(
        &mut socket,
        1,
        "initialize",
        json!({
            "clientInfo": {"name": "aoe-recovery", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }),
    )
    .await?;
    socket
        .send(Message::Text(
            json!({"method": "initialized"}).to_string().into(),
        ))
        .await?;
    let result = request(
        &mut socket,
        2,
        "thread/read",
        json!({"threadId": thread, "includeTurns": false}),
    )
    .await?;
    validate(result, thread, cwd)?;
    socket
        .close(None)
        .await
        .context("Closing Codex recovery verification")?;
    Ok(())
}

fn validate(value: serde_json::Value, thread: &str, cwd: &str) -> Result<()> {
    let response: Response =
        serde_json::from_value(value).context("Invalid Codex recovery thread response")?;
    ensure!(
        response.thread.id == thread,
        "Codex returned a different thread"
    );
    ensure!(
        response.thread.status.kind == "notLoaded",
        "Original Codex thread is still loaded; refuse explicit recovery"
    );
    let original = std::fs::canonicalize(&response.thread.cwd)
        .context("Resolving original Codex working directory")?;
    let target = std::fs::canonicalize(cwd).context("Resolving recovery pane directory")?;
    ensure!(
        original == target,
        "Original Codex thread belongs to a different directory"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_requires_the_selected_unloaded_thread_and_exact_directory() {
        let directory = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let cwd = directory.path().to_str().unwrap();
        let response = json!({"thread": {
            "id": "original", "cwd": cwd, "status": {"type": "notLoaded"}
        }});
        assert!(validate(response.clone(), "original", cwd).is_ok());
        assert!(validate(response.clone(), "another", cwd).is_err());
        assert!(validate(response.clone(), "original", other.path().to_str().unwrap()).is_err());
        for status in ["active", "idle", "systemError"] {
            let mut loaded = response.clone();
            loaded["thread"]["status"]["type"] = json!(status);
            assert!(validate(loaded, "original", cwd).is_err());
        }
        assert!(validate(json!({}), "original", cwd).is_err());
    }
}
