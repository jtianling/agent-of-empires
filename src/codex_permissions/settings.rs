use anyhow::{ensure, Context, Result};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::db::runtime_settings::RuntimeSettings;

#[cfg(test)]
mod tests;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsResponse {
    thread: super::Thread,
    model: String,
    reasoning_effort: Option<String>,
    service_tier: Option<String>,
}

pub(crate) fn capture_persisted(
    thread: &str,
    expected_cwd: &Path,
) -> Result<Option<RuntimeSettings>> {
    uuid::Uuid::parse_str(thread).context("Invalid persisted Codex thread ID")?;
    let Some((path, cwd)) = super::history::indexed_rollout(thread)? else {
        return Ok(None);
    };
    ensure!(
        cwd.canonicalize()? == expected_cwd.canonicalize()?,
        "Codex settings index directory mismatch"
    );
    read_persisted_at(&path, thread, expected_cwd)
}

// Match the rollout discovery header limit. Later records can include inline media;
// allow one default app-server WebSocket message per JSONL record.
const MAX_HEADER_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn read_persisted_at(
    path: &Path,
    thread: &str,
    expected_cwd: &Path,
) -> Result<Option<RuntimeSettings>> {
    uuid::Uuid::parse_str(thread).context("Invalid expected Codex thread UUID")?;
    let path = super::history::physical_rollout(path)?;
    let file = std::fs::File::open(&path).context("Opening Codex settings history")?;
    let reader: Box<dyn Read> = if path.extension().is_some_and(|ext| ext == "zst") {
        Box::new(zstd::Decoder::new(file).context("Opening compressed Codex history")?)
    } else {
        Box::new(file)
    };
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    let header = read_record(&mut reader, &mut buffer, 1, MAX_HEADER_BYTES)?
        .context("Missing Codex history metadata at line 1")?;
    validate_header(header, thread, expected_cwd)
        .context("Invalid Codex history metadata at line 1")?;
    let mut latest = None;
    for number in 2.. {
        let Some(line) = read_record(&mut reader, &mut buffer, number, MAX_RECORD_BYTES)? else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str(line)
            .with_context(|| format!("Invalid JSON in Codex settings history at line {number}"))?;
        if let Some(settings) = settings_from_record(&value, thread)
            .with_context(|| format!("Invalid Codex settings at line {number}"))?
        {
            latest = Some(settings);
        }
    }
    Ok(latest)
}

fn read_record<'a>(
    reader: &mut impl BufRead,
    buffer: &'a mut Vec<u8>,
    number: usize,
    limit: usize,
) -> Result<Option<&'a str>> {
    buffer.clear();
    let bytes = reader
        .take(limit as u64 + 2)
        .read_until(b'\n', buffer)
        .with_context(|| format!("Reading Codex history at line {number}"))?;
    if bytes == 0 {
        return Ok(None);
    }
    let record = buffer
        .strip_suffix(b"\r\n")
        .or_else(|| buffer.strip_suffix(b"\n"))
        .unwrap_or(buffer);
    ensure!(
        record.len() <= limit,
        "Codex history line {number} exceeds the {limit}-byte record limit"
    );
    let line = std::str::from_utf8(record)
        .with_context(|| format!("Invalid UTF-8 in Codex history at line {number}"))?;
    Ok(Some(line))
}

fn validate_header(line: &str, thread: &str, cwd: &Path) -> Result<()> {
    #[derive(Deserialize)]
    struct Header {
        #[serde(rename = "type")]
        kind: String,
        payload: Meta,
    }
    #[derive(Deserialize)]
    struct Meta {
        #[serde(default, deserialize_with = "present_id")]
        id: Option<String>,
        #[serde(default, deserialize_with = "present_id")]
        session_id: Option<String>,
        cwd: std::path::PathBuf,
    }
    fn present_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        String::deserialize(d).map(Some)
    }
    let header: Header = serde_json::from_str(line)?;
    ensure!(
        header.kind == "session_meta",
        "Expected session_meta record"
    );
    let meta = header.payload;
    if let (Some(id), Some(session_id)) = (&meta.id, &meta.session_id) {
        ensure!(
            id == session_id,
            "Conflicting Codex metadata id and session_id"
        );
    }
    let owner = meta
        .id
        .or(meta.session_id)
        .context("Missing Codex metadata id or session_id")?;
    ensure!(
        owner == thread,
        "Codex history metadata belongs to a different thread"
    );
    ensure!(
        meta.cwd
            .canonicalize()
            .context("Resolving recorded Codex directory")?
            == cwd
                .canonicalize()
                .context("Resolving expected Codex directory")?,
        "Codex history belongs to a different directory"
    );
    Ok(())
}

fn settings_from_record(
    value: &serde_json::Value,
    thread: &str,
) -> Result<Option<RuntimeSettings>> {
    if value.get("type").and_then(|v| v.as_str()) != Some("event_msg")
        || value.pointer("/payload/type").and_then(|v| v.as_str())
            != Some("thread_settings_applied")
    {
        return Ok(None);
    }
    if value.pointer("/payload/thread_id").and_then(|v| v.as_str()) != Some(thread) {
        return Ok(None);
    }
    #[derive(Deserialize)]
    struct Snapshot {
        model: String,
        reasoning_effort: Option<String>,
        service_tier: Option<String>,
    }
    let snapshot: Snapshot = serde_json::from_value(
        value
            .pointer("/payload/thread_settings")
            .context("Codex settings history has no snapshot")?
            .clone(),
    )
    .context("Invalid persisted Codex settings")?;
    let settings = RuntimeSettings {
        model: snapshot.model,
        effort: snapshot.reasoning_effort,
        service_tier: snapshot.service_tier,
    };
    settings.validate("codex")?;
    Ok(Some(settings))
}

pub(crate) fn capture(endpoint: String, thread: String) -> Result<Option<RuntimeSettings>> {
    uuid::Uuid::parse_str(&thread).context("Invalid Codex settings thread ID")?;
    crate::xats_control::block_on_control("codex-settings", move || {
        Box::pin(async move {
            tokio::time::timeout(super::TIMEOUT, read(&endpoint, &thread))
                .await
                .context("Timed out reading Codex pane settings")?
        })
    })
}

async fn read(endpoint: &str, thread: &str) -> Result<Option<RuntimeSettings>> {
    let (mut socket, _) = connect_async(endpoint)
        .await
        .context("Connecting to Codex for pane settings")?;
    super::request(
        &mut socket,
        1,
        "initialize",
        json!({
            "clientInfo": {"name": "aoe-settings", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }),
    )
    .await?;
    socket
        .send(Message::Text(
            json!({"method": "initialized"}).to_string().into(),
        ))
        .await
        .context("Initializing Codex settings client")?;
    let response = super::request(
        &mut socket,
        2,
        "thread/read",
        json!({"threadId": thread, "includeTurns": false}),
    )
    .await?;
    ensure!(
        response.pointer("/thread/id").and_then(|id| id.as_str()) == Some(thread),
        "Codex settings response belongs to another thread"
    );
    // Rejoining a loaded thread returns its live settings without overrides.
    // Never load an idle history using defaults from another pane.
    let status = response
        .pointer("/thread/status/type")
        .and_then(|v| v.as_str())
        .context("Codex did not report its thread status")?;
    ensure!(
        matches!(status, "notLoaded" | "idle" | "active" | "systemError"),
        "Codex returned an unknown thread status: {status}"
    );
    let settings = if status == "notLoaded" {
        match response
            .pointer("/thread/path")
            .and_then(|path| path.as_str())
        {
            Some(path) => {
                let cwd = response
                    .pointer("/thread/cwd")
                    .and_then(|v| v.as_str())
                    .context("Codex settings response has no working directory")?;
                read_persisted_at(Path::new(path), thread, Path::new(cwd))?
            }
            None => None,
        }
    } else {
        let response = super::request(
            &mut socket,
            3,
            "thread/resume",
            json!({"threadId": thread, "excludeTurns": true}),
        )
        .await?;
        ensure!(
            response.get("serviceTier").is_some(),
            "Codex did not expose its service tier"
        );
        ensure!(
            response.get("reasoningEffort").is_some(),
            "Codex did not expose its reasoning effort"
        );
        let response: SettingsResponse =
            serde_json::from_value(response).context("Invalid Codex pane settings response")?;
        ensure!(
            response.thread.id == thread,
            "Codex resumed another settings thread"
        );
        let settings = RuntimeSettings {
            model: response.model,
            effort: response.reasoning_effort,
            service_tier: response.service_tier,
        };
        settings.validate("codex")?;
        Some(settings)
    };
    socket
        .close(None)
        .await
        .context("Closing Codex settings client")?;
    Ok(settings)
}
