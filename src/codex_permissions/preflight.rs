use anyhow::{ensure, Context, Result};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::{request, TIMEOUT};

#[derive(Deserialize)]
struct Response {
    thread: Thread,
}

#[derive(Deserialize)]
struct Thread {
    id: String,
    cwd: String,
    path: Option<PathBuf>,
}

pub(crate) fn preflight_resume(endpoint: &str, thread: &str, cwd: &str) -> Result<()> {
    inspect_source(endpoint, thread, cwd).map(drop)
}

pub(crate) fn preflight_settings(
    endpoint: &str,
    thread: &str,
    cwd: &str,
) -> Result<Option<crate::db::runtime_settings::RuntimeSettings>> {
    let (rollout, sqlite_home) = inspect_source(endpoint, thread, cwd)?;
    if !has_local_history(endpoint) {
        return Ok(None);
    }
    if let Some(path) = rollout {
        return super::settings::read_persisted_at(&path, thread, Path::new(cwd));
    }
    if let Some(dir) = sqlite_home {
        ensure!(dir.is_absolute(), "Codex sqlite_home must be absolute");
        let Some((path, indexed_cwd)) = super::history::indexed_in(&dir, thread)? else {
            return Ok(None);
        };
        ensure!(
            indexed_cwd.canonicalize()? == Path::new(cwd).canonicalize()?,
            "Codex settings index directory mismatch"
        );
        return super::settings::read_persisted_at(&path, thread, Path::new(cwd));
    }
    super::settings::capture_persisted(thread, Path::new(cwd))
}

fn has_local_history(endpoint: &str) -> bool {
    super::is_local(endpoint)
        || url::Url::parse(endpoint).is_ok_and(|url| {
            url.host_str()
                .is_some_and(|host| host.trim_end_matches('.').eq_ignore_ascii_case("localhost"))
        })
}

fn inspect_source(
    endpoint: &str,
    thread: &str,
    cwd: &str,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    uuid::Uuid::parse_str(thread).context("Invalid Codex resume UUID")?;
    let url = url::Url::parse(endpoint).context("Invalid Codex resume endpoint")?;
    ensure!(
        matches!(url.scheme(), "ws" | "wss") && url.host_str().is_some(),
        "Codex resume requires a WebSocket endpoint"
    );
    let (endpoint, thread, cwd) = (endpoint.to_owned(), thread.to_owned(), cwd.to_owned());
    crate::xats_control::block_on_control("codex-resume-preflight", move || {
        Box::pin(async move {
            tokio::time::timeout(TIMEOUT, inspect(&endpoint, &thread, &cwd))
                .await
                .context("Timed out checking Codex resume source")?
        })
    })
}

async fn inspect(
    endpoint: &str,
    thread: &str,
    cwd: &str,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    let (mut socket, _) = connect_async(endpoint)
        .await
        .context("Connecting to Codex for resume preflight")?;
    request(
        &mut socket,
        1,
        "initialize",
        json!({
            "clientInfo": {"name": "aoe-resume-preflight", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }),
    )
    .await?;
    socket
        .send(Message::Text(
            json!({"method": "initialized"}).to_string().into(),
        ))
        .await?;
    let config = request(
        &mut socket,
        2,
        "config/read",
        json!({
            "includeLayers": false, "cwd": cwd
        }),
    )
    .await?;
    let sqlite_home = config
        .pointer("/config/sqlite_home")
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_str()
                .map(PathBuf::from)
                .context("Invalid Codex sqlite_home")
        })
        .transpose()?;
    super::current_provider(config)?;
    let value = request(
        &mut socket,
        3,
        "thread/read",
        json!({
            "threadId": thread, "includeTurns": false
        }),
    )
    .await?;
    let path = validate(value, thread, cwd)?;
    socket
        .close(None)
        .await
        .context("Closing Codex resume preflight")?;
    Ok((path, sqlite_home))
}

fn validate(value: serde_json::Value, thread: &str, cwd: &str) -> Result<Option<PathBuf>> {
    let response: Response =
        serde_json::from_value(value).context("Invalid Codex resume preflight response")?;
    ensure!(
        response.thread.id == thread,
        "Codex returned a different resume thread"
    );
    let original = std::fs::canonicalize(&response.thread.cwd)
        .context("Resolving original Codex directory")?;
    let target = std::fs::canonicalize(cwd).context("Resolving Codex resume directory")?;
    ensure!(
        original == target,
        "Original Codex thread belongs to a different directory"
    );
    Ok(response.thread.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_paths_belong_to_the_local_app_server() {
        assert!(has_local_history("ws://127.0.0.1:8799"));
        assert!(has_local_history("ws://[::1]:8799"));
        assert!(has_local_history("ws://localhost:8899"));
        assert!(!has_local_history("wss://remote.example"));
    }

    #[test]
    fn preflight_accepts_loaded_original_and_rejects_wrong_thread_or_directory() {
        let directory = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let cwd = directory.path().to_str().unwrap();
        let value = json!({"thread": {"id": "original", "cwd": cwd,
            "status": {"type": "active"}}});
        assert!(validate(value.clone(), "original", cwd).is_ok());
        assert!(validate(value.clone(), "another", cwd).is_err());
        assert!(validate(value, "original", other.path().to_str().unwrap()).is_err());
    }
}
