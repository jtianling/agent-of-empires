use anyhow::{bail, ensure, Context, Result};
use futures_util::SinkExt;
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadedThreads {
    data: Vec<String>,
    next_cursor: Option<String>,
}

#[cfg(test)]
mod tests;

pub(super) fn retire(previous: &super::ClientConfig) -> Result<()> {
    inspect(previous, true)
}

pub(super) fn preflight(previous: &super::ClientConfig) -> Result<()> {
    inspect(previous, false)
}

fn inspect(previous: &super::ClientConfig, wait_unload: bool) -> Result<()> {
    if previous.owned || previous.thread_id.is_empty() {
        return Ok(());
    }
    super::validate_endpoint(&previous.ws_url)?;
    uuid::Uuid::try_parse(&previous.thread_id).context("Invalid legacy managed thread ID")?;
    let previous = previous.clone();
    crate::xats_control::block_on_control("managed-codex-migration", move || {
        Box::pin(async move {
            let timeout = super::Duration::from_secs(if wait_unload { 105 } else { 15 });
            tokio::time::timeout(timeout, retire_thread(&previous, wait_unload))
                .await
                .context("Timed out retiring legacy managed Codex runtime")?
        })
    })
}

async fn retire_thread(previous: &super::ClientConfig, wait_unload: bool) -> Result<()> {
    let (mut socket, _) = match connect_async(&previous.ws_url).await {
        Ok(socket) => socket,
        Err(tokio_tungstenite::tungstenite::Error::Io(error))
            if error.kind() == std::io::ErrorKind::ConnectionRefused =>
        {
            tracing::info!("Legacy managed Codex endpoint is offline: {error}");
            return Ok(());
        }
        Err(error) => return Err(error).context("Connecting to legacy managed Codex runtime"),
    };
    let result = inspect_thread(&mut socket, &previous.thread_id, wait_unload).await;
    let closed = socket
        .close(None)
        .await
        .context("Closing legacy managed Codex connection");
    match (result, closed) {
        (Err(error), Err(closed)) => bail!("{error:#}; {closed:#}"),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

async fn inspect_thread(
    socket: &mut crate::codex_permissions::Socket,
    thread: &str,
    wait_unload: bool,
) -> Result<()> {
    super::request(
        socket,
        1,
        "initialize",
        json!({
            "clientInfo": {"name": "aoe-managed-migration", "version": env!("CARGO_PKG_VERSION")}
        }),
    )
    .await?;
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"method": "initialized"}).to_string().into(),
        ))
        .await?;
    let status = read_status(socket, thread, 2).await?;
    ensure!(
        matches!(status.as_str(), "idle" | "notLoaded" | "systemError"),
        "Cannot migrate active legacy managed Codex thread ({status})"
    );
    if !wait_unload || status == "notLoaded" {
        return Ok(());
    }
    let resumed = super::request(
        socket,
        3,
        "thread/resume",
        json!({"threadId": thread, "excludeTurns": true,
            "config": {"thread_unload_delay_secs": 0}}),
    )
    .await?;
    let status = response_status(&resumed, thread)?;
    ensure!(
        matches!(status, "idle" | "notLoaded" | "systemError"),
        "Legacy Codex thread became active during migration ({status})"
    );
    super::unsubscribe_previous(socket, thread).await?;
    await_unloaded(socket, thread).await
}

async fn read_status(
    socket: &mut crate::codex_permissions::Socket,
    thread: &str,
    id: u64,
) -> Result<String> {
    let response = super::request(
        socket,
        id,
        "thread/read",
        json!({
            "threadId": thread, "includeTurns": false
        }),
    )
    .await?;
    Ok(response_status(&response, thread)?.to_string())
}

fn response_status<'value>(response: &'value Value, thread: &str) -> Result<&'value str> {
    ensure!(
        response.pointer("/thread/id").and_then(Value::as_str) == Some(thread),
        "Legacy Codex runtime returned another thread"
    );
    response
        .pointer("/thread/status/type")
        .and_then(Value::as_str)
        .context("Legacy Codex thread has no status")
}

async fn await_unloaded(socket: &mut crate::codex_permissions::Socket, thread: &str) -> Result<()> {
    let mut status = String::new();
    let result = tokio::time::timeout(super::Duration::from_secs(90), async {
        let mut request_id = 20;
        loop {
            let response =
                super::request(socket, request_id, "thread/loaded/list", json!({})).await?;
            let loaded: LoadedThreads =
                serde_json::from_value(response).context("Invalid legacy loaded-thread list")?;
            ensure!(
                loaded.next_cursor.is_none(),
                "Legacy loaded-thread list unexpectedly paginated"
            );
            for id in &loaded.data {
                uuid::Uuid::try_parse(id).context("Invalid loaded legacy thread ID")?;
            }
            if !loaded.data.iter().any(|id| id == thread) {
                return Ok(());
            }
            status = "loaded".into();
            request_id += 1;
            tokio::time::sleep(super::Duration::from_millis(500)).await;
        }
    })
    .await;
    result.with_context(|| {
        format!("Legacy Codex thread {thread} did not unload (last status: {status})")
    })?
}
