use anyhow::{ensure, Context, Result};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::command::{Command, Intent};
use crate::codex_permissions::{current_provider, request, validate_resume, Socket};

#[derive(Deserialize)]
struct Created {
    thread: Thread,
}

#[derive(Deserialize)]
struct Thread {
    id: String,
    cwd: String,
}

pub(super) fn settings(response: &Value) -> Result<crate::db::runtime_settings::RuntimeSettings> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Settings {
        model: String,
        reasoning_effort: Option<String>,
        service_tier: Option<String>,
    }
    let value: Settings = serde_json::from_value(response.clone())
        .context("Invalid created Codex thread settings")?;
    let settings = crate::db::runtime_settings::RuntimeSettings {
        model: value.model,
        effort: value.reasoning_effort,
        service_tier: value.service_tier,
    };
    settings.validate("codex")?;
    Ok(settings)
}

pub(super) async fn connect(endpoint: &str) -> Result<Socket> {
    let (mut socket, _) = connect_async(endpoint)
        .await
        .context("Connecting Codex launcher")?;
    request(
        &mut socket,
        1,
        "initialize",
        json!({
            "clientInfo": {"name": "aoe-launcher", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }),
    )
    .await?;
    socket
        .send(Message::Text(
            json!({"method":"initialized"}).to_string().into(),
        ))
        .await?;
    Ok(socket)
}

pub(super) async fn create(
    socket: &mut Socket,
    command: &Command,
    cwd: &std::path::Path,
) -> Result<(String, String, Value)> {
    let configuration = request(
        socket,
        2,
        "config/read",
        json!({
            "includeLayers": false, "cwd": cwd
        }),
    )
    .await?;
    let provider = command
        .config
        .get("model_provider")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or(current_provider(configuration)?);
    ensure!(!provider.trim().is_empty(), "Codex provider is empty");
    let (method, params) = creation_parameters(command, cwd, &provider);
    let response = request(socket, 3, method, params).await?;
    let created: Created = serde_json::from_value(response.clone())
        .context("Invalid Codex created-thread response")?;
    uuid::Uuid::parse_str(&created.thread.id).context("Invalid created Codex UUID")?;
    if let Intent::Fork(parent) = &command.intent {
        ensure!(
            created.thread.id != *parent,
            "Codex fork returned its parent UUID"
        );
    }
    ensure!(
        std::fs::canonicalize(&created.thread.cwd)? == cwd,
        "Created Codex thread belongs to a different directory"
    );
    Ok((created.thread.id, provider, response))
}

fn creation_parameters(
    command: &Command,
    cwd: &std::path::Path,
    provider: &str,
) -> (&'static str, Value) {
    let mut params = json!({"cwd": cwd, "modelProvider": provider,
        "config": command.config, "ephemeral": false});
    for (key, value) in [
        ("model", command.model.as_ref()),
        ("approvalPolicy", command.approval.as_ref()),
        ("sandbox", command.sandbox.as_ref()),
        ("approvalsReviewer", command.reviewer.as_ref()),
    ] {
        if let Some(value) = value {
            params[key] = json!(value);
        }
    }
    if let Some(tier) = command.config.get("service_tier") {
        params["serviceTier"] = tier.clone();
    }
    let method = if let Intent::Fork(parent) = &command.intent {
        params["threadId"] = json!(parent);
        params["excludeTurns"] = json!(true);
        "thread/fork"
    } else {
        "thread/start"
    };
    (method, params)
}

pub(super) async fn finish(
    socket: &mut Socket,
    command: &Command,
    thread: &str,
    provider: &str,
    response: Value,
    title: &str,
) -> Result<()> {
    validate_resume(
        response.clone(),
        thread,
        provider,
        command.sandbox.as_deref() == Some("danger-full-access"),
    )?;
    validate_settings(&response, command)?;
    request(
        socket,
        4,
        "thread/name/set",
        json!({"threadId": thread, "name": title}),
    )
    .await?;
    let saved = request(
        socket,
        5,
        "thread/read",
        json!({
            "threadId": thread, "includeTurns": false
        }),
    )
    .await?;
    ensure!(
        saved.pointer("/thread/id").and_then(Value::as_str) == Some(thread),
        "Codex persisted a different thread"
    );
    ensure!(
        saved
            .pointer("/thread/path")
            .and_then(Value::as_str)
            .is_some(),
        "Codex created thread has no durable history path"
    );
    if command.sandbox.as_deref() == Some("danger-full-access") {
        // Remote UI resume uses saved thread settings for its first turn.
        request(
            socket,
            6,
            "thread/settings/update",
            json!({"threadId": thread, "approvalPolicy": "never",
                   "sandboxPolicy": {"type": "dangerFullAccess"}}),
        )
        .await
        .context("Saving YOLO permissions before the Codex UI attaches")?;
    }
    socket
        .close(None)
        .await
        .context("Closing Codex launcher preparation")
}

fn validate_settings(response: &Value, command: &Command) -> Result<()> {
    if command.sandbox.as_deref() == Some("workspace-write") {
        ensure!(
            response.pointer("/sandbox/type").and_then(Value::as_str) == Some("workspaceWrite")
                && response.get("approvalPolicy").and_then(Value::as_str) == Some("on-request"),
            "Codex did not retain requested workspace-write permissions"
        );
    }
    let model = command
        .model
        .as_deref()
        .or_else(|| command.config.get("model").and_then(Value::as_str));
    if let Some(model) = model {
        ensure!(
            response.get("model").and_then(Value::as_str) == Some(model),
            "Codex did not retain the requested model"
        );
    }
    for (configuration, field) in [
        ("model_reasoning_effort", "reasoningEffort"),
        ("service_tier", "serviceTier"),
    ] {
        if let Some(expected) = command.config.get(configuration).and_then(Value::as_str) {
            let actual = response.get(field).and_then(Value::as_str);
            ensure!(
                actual == Some(expected)
                    || (configuration == "service_tier"
                        && expected == "default"
                        && actual.is_none()),
                "Codex did not retain requested {configuration}"
            );
        }
    }
    if let Some(reviewer) = &command.reviewer {
        ensure!(
            response.get("approvalsReviewer").and_then(Value::as_str) == Some(reviewer),
            "Codex did not retain the requested approvals reviewer"
        );
    }
    Ok(())
}
