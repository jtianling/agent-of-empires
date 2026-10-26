use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::codex_permissions::request;
use crate::session::Instance;

const HEADER: &str = "X-Xats-Managed-Token";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientConfig {
    thread_id: String,
    ws_url: String,
    profile_name: String,
    profile_path: PathBuf,
    #[serde(default)]
    fresh: bool,
}

pub(crate) fn prepare(instance: &mut Instance) -> Result<()> {
    if instance.tool != "codex" || instance.managed_xats.is_none() {
        return Ok(());
    }
    let token = crate::managed_xats::mcp_token(instance)?.to_string();
    let previous = client_config(instance)?;
    let endpoint = url::Url::parse(&crate::session::managed_codex_endpoint()?)?.to_string();
    validate_endpoint(&endpoint)?;
    if let Some(previous) = &previous {
        ensure!(
            url::Url::parse(&previous.ws_url)? == url::Url::parse(&endpoint)?,
            "Managed Codex app-server changed"
        );
    }
    let mcp_url = crate::xats_control::discover_control_plane()?
        .base_url
        .join("/mcp")?;
    let cwd = instance.project_path.clone();
    let title = instance.title.clone();
    let previous_thread = previous.as_ref().map(|config| config.thread_id.clone());
    let fresh = previous.as_ref().is_some_and(|config| config.fresh);
    let mut config = profile_config(instance, &endpoint, previous.as_ref())?;
    let rpc_endpoint = endpoint.clone();
    let rpc_token = token.clone();
    let mut prepared = crate::xats_control::block_on_control("managed-codex", move || {
        Box::pin(async move {
            tokio::time::timeout(
                Duration::from_secs(20),
                prepare_thread(
                    &rpc_endpoint,
                    &mcp_url,
                    &cwd,
                    &title,
                    &rpc_token,
                    previous_thread.as_deref(),
                    fresh,
                ),
            )
            .await
            .context("Timed out preparing managed Codex thread")?
        })
    })
    .map_err(|error| anyhow::anyhow!(format!("{error:#}").replace(&token, "[redacted]")))?;
    config.thread_id = prepared.thread_id.clone();
    let runtime = json!({"agent_type": "codex", "thread_id": config.thread_id,
        "ws_url": config.ws_url});
    instance.agent_session_id = Some(config.thread_id.clone());
    instance
        .managed_xats
        .as_mut()
        .context("Missing managed identity")?
        .client_config = Some(serde_json::to_value(&config)?);
    if let Some(error) = prepared.error.take() {
        bail!(format!("{error:#}").replace(&token, "[redacted]"));
    }
    write_profile(&config, &prepared)?;
    crate::managed_xats::bind(instance, runtime)
}

struct PreparedThread {
    thread_id: String,
    server_name: String,
    headers: Value,
    error: Option<anyhow::Error>,
}

async fn prepare_thread(
    endpoint: &str,
    mcp_url: &url::Url,
    cwd: &str,
    title: &str,
    token: &str,
    previous: Option<&str>,
    fresh: bool,
) -> Result<PreparedThread> {
    let (mut socket, _) = connect_async(endpoint)
        .await
        .context("Connecting to Codex")?;
    request(
        &mut socket,
        1,
        "initialize",
        json!({
            "clientInfo": {"name": "aoe-managed", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }),
    )
    .await?;
    socket
        .send(Message::Text(
            json!({"method": "initialized"}).to_string().into(),
        ))
        .await?;
    let configuration = request(
        &mut socket,
        2,
        "config/read",
        json!({"includeLayers": false, "cwd": cwd}),
    )
    .await?;
    let (server_name, headers) = mcp_headers(&configuration, mcp_url, token)?;
    let override_key = format!("mcp_servers.{server_name}.http_headers");
    let url_key = format!("mcp_servers.{server_name}.url");
    let config = json!({(override_key): headers, (url_key): mcp_url.as_str()});
    if let Some(id) = previous {
        uuid::Uuid::parse_str(id).context("Invalid stored managed Codex thread")?;
        unsubscribe_previous(&mut socket, id, cwd).await?;
    }
    let resumed_thread = previous.filter(|_| !fresh);
    let (method, params) = match resumed_thread {
        Some(id) => (
            "thread/resume",
            json!({"threadId": id, "cwd": cwd,
            "config": config, "excludeTurns": true}),
        ),
        None => ("thread/start", json!({"cwd": cwd, "config": config})),
    };
    let response = request(&mut socket, 3, method, params).await?;
    let thread_id = response
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .context("Codex response has no exact thread ID")?
        .to_string();
    uuid::Uuid::parse_str(&thread_id).context("Invalid Codex thread ID")?;
    ensure!(
        resumed_thread.map_or(true, |id| id == thread_id),
        "Codex resumed a different thread"
    );
    let persisted = request(
        &mut socket,
        4,
        "thread/name/set",
        json!({"threadId": thread_id, "name": title}),
    )
    .await;
    let closed = socket
        .close(None)
        .await
        .context("Closing managed Codex connection");
    let error = match (persisted.err(), closed.err()) {
        (Some(first), Some(second)) => Some(anyhow::anyhow!("{first:#}; {second:#}")),
        (Some(error), None) | (None, Some(error)) => Some(error),
        (None, None) => None,
    };
    Ok(PreparedThread {
        thread_id,
        server_name,
        headers,
        error,
    })
}

async fn unsubscribe_previous(
    socket: &mut crate::codex_permissions::Socket,
    id: &str,
    cwd: &str,
) -> Result<()> {
    let resumed = request(
        socket,
        10,
        "thread/resume",
        json!({"threadId": id, "cwd": cwd, "excludeTurns": true}),
    )
    .await?;
    ensure!(
        resumed.pointer("/thread/id").and_then(Value::as_str) == Some(id),
        "Codex subscribed to a different thread"
    );
    let result = request(socket, 11, "thread/unsubscribe", json!({"threadId": id})).await?;
    ensure!(
        matches!(
            result.get("status").and_then(Value::as_str),
            Some("unsubscribed" | "notLoaded")
        ),
        "Codex did not unsubscribe the managed thread"
    );
    Ok(())
}

fn mcp_headers(config: &Value, url: &url::Url, token: &str) -> Result<(String, Value)> {
    let servers = config
        .pointer("/config/mcp_servers")
        .and_then(Value::as_object)
        .context("Codex has no configured MCP servers")?;
    let matches: Vec<_> = servers
        .iter()
        .filter(|(_, value)| {
            value
                .get("url")
                .and_then(Value::as_str)
                .and_then(|url| url::Url::parse(url).ok())
                .as_ref()
                == Some(url)
                && value.get("enabled").and_then(Value::as_bool) != Some(false)
        })
        .collect();
    ensure!(
        matches.len() == 1,
        "Expected exactly one enabled Codex xats MCP endpoint"
    );
    let (name, server) = matches[0];
    ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
        "Invalid xats MCP server name"
    );
    let mut headers = server
        .get("http_headers")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let object = headers
        .as_object_mut()
        .context("Invalid xats MCP headers")?;
    ensure!(
        object.values().all(Value::is_string),
        "Invalid xats MCP header value"
    );
    object.insert(HEADER.into(), json!(token));
    Ok((name.clone(), headers))
}

fn profile_config(
    instance: &Instance,
    endpoint: &str,
    previous: Option<&ClientConfig>,
) -> Result<ClientConfig> {
    ensure!(
        instance.primary_pane_id().len() == 16
            && instance
                .primary_pane_id()
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "Invalid managed session ID"
    );
    let home = match std::env::var_os("CODEX_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        Some(_) => bail!("CODEX_HOME must not be empty"),
        None => dirs::home_dir()
            .context("Missing home directory")?
            .join(".codex"),
    };
    let profile_name = format!("aoe-managed-{}", instance.primary_pane_id());
    let profile_path = home.join(format!("{profile_name}.config.toml"));
    if profile_path.exists() {
        ensure!(
            previous.is_some_and(|previous| previous.profile_path == profile_path
                && previous.profile_name == profile_name),
            "Managed Codex profile already exists"
        );
    }
    fs::create_dir_all(&home)?;
    Ok(ClientConfig {
        thread_id: String::new(),
        ws_url: endpoint.into(),
        profile_name,
        profile_path,
        fresh: false,
    })
}

fn write_profile(config: &ClientConfig, prepared: &PreparedThread) -> Result<()> {
    let value = json!({"mcp_servers": {(prepared.server_name.clone()): {
        "http_headers": prepared.headers
    }}});
    let body = toml::to_string(&value).context("Serializing managed Codex profile")?;
    write_private(&config.profile_path, body.as_bytes())
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        if let Err(error) = fs::remove_file(&temporary) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Failed to remove managed Codex temporary profile: {error}");
            }
        }
    }
    result
}

fn client_config(instance: &Instance) -> Result<Option<ClientConfig>> {
    instance
        .managed_xats
        .as_ref()
        .and_then(|xats| xats.client_config.clone())
        .map(|value| {
            serde_json::from_value(value).context("Invalid managed Codex client configuration")
        })
        .transpose()
}

pub(crate) fn command(instance: &Instance, base: &str, cwd: &str) -> Result<String> {
    use crate::session::environment::shell_escape;
    let config = client_config(instance)?.context("Managed Codex has not been prepared")?;
    let runtime = crate::managed_xats::runtime(instance).context("Managed Codex is not bound")?;
    ensure!(
        runtime.get("thread_id").and_then(Value::as_str) == Some(&config.thread_id)
            && runtime.get("ws_url").and_then(Value::as_str) == Some(&config.ws_url),
        "Managed Codex binding does not match its profile"
    );
    validate_endpoint(&config.ws_url)?;
    uuid::Uuid::parse_str(&config.thread_id).context("Invalid managed Codex thread")?;
    let expected_profile = format!("aoe-managed-{}", instance.primary_pane_id());
    ensure!(
        config.profile_name == expected_profile,
        "Invalid managed Codex profile"
    );
    let permission = if instance.primary_pane.yolo_mode {
        " --dangerously-bypass-approvals-and-sandbox"
    } else {
        ""
    };
    let command = format!(
        "{base} --remote {} -C {} -p {}{permission} resume {}",
        shell_escape(&config.ws_url),
        shell_escape(cwd),
        shell_escape(&config.profile_name),
        shell_escape(&config.thread_id)
    );
    crate::managed_runtimes::wrap_pane_command(instance, &command)
}

pub(crate) fn cleanup(instance: &Instance) -> Result<()> {
    let Some(config) = client_config(instance)? else {
        return Ok(());
    };
    let expected = format!("aoe-managed-{}.config.toml", instance.primary_pane_id());
    ensure!(
        config
            .profile_path
            .file_name()
            .and_then(|name| name.to_str())
            == Some(&expected),
        "Invalid managed Codex profile path"
    );
    match fs::remove_file(config.profile_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Removing managed Codex profile"),
    }
}

fn validate_endpoint(endpoint: &str) -> Result<()> {
    let url = url::Url::parse(endpoint)?;
    ensure!(
        matches!(url.scheme(), "ws" | "wss")
            && url.username().is_empty()
            && url.password().is_none(),
        "Invalid managed Codex endpoint"
    );
    let local = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    ensure!(local, "Managed Codex requires a local app-server");
    Ok(())
}

pub(crate) fn mark_fresh(instance: &mut Instance) -> Result<()> {
    if instance.tool != "codex" {
        return Ok(());
    }
    if let Some(mut config) = client_config(instance)? {
        config.fresh = true;
        instance
            .managed_xats
            .as_mut()
            .context("Missing managed identity")?
            .client_config = Some(serde_json::to_value(config)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_unique_exact_mcp_endpoint_and_preserves_headers() {
        let url = url::Url::parse("http://127.0.0.1:1234/mcp").unwrap();
        let config = json!({"config": {"mcp_servers": {
            "xats": {"url": url.as_str(), "http_headers": {"Existing": "value"}},
            "other": {"url": "http://127.0.0.1:4321/mcp"}
        }}});
        let (name, headers) = mcp_headers(&config, &url, "private-token").unwrap();
        assert_eq!(name, "xats");
        assert_eq!(headers["Existing"], "value");
        assert_eq!(headers[HEADER], "private-token");
        let duplicate = json!({"config": {"mcp_servers": {
            "one": {"url": url.as_str()}, "two": {"url": url.as_str()}
        }}});
        assert!(mcp_headers(&duplicate, &url, "token").is_err());
    }

    #[test]
    fn launch_command_references_profile_without_token() {
        let mut instance = Instance::new("worker", "/tmp");
        instance.tool = "codex".into();
        let thread = uuid::Uuid::new_v4().to_string();
        let mut identity =
            crate::managed_xats::ManagedXats::new("worker".into(), "team".into()).unwrap();
        identity.claim = Some(
            serde_json::from_value(json!({
                "fence": {"claim_id": uuid::Uuid::new_v4().to_string(),
                    "instance_id": identity.launch_instance_id, "generation": 1},
                "mcp_token": "private-token",
                "runtime": {"agent_type": "codex", "thread_id": thread,
                    "ws_url": "ws://127.0.0.1:8799"}
            }))
            .unwrap(),
        );
        let profile_name = format!("aoe-managed-{}", instance.id);
        identity.client_config = Some(
            serde_json::to_value(ClientConfig {
                thread_id: thread.clone(),
                ws_url: "ws://127.0.0.1:8799".into(),
                profile_name: profile_name.clone(),
                profile_path: PathBuf::from("unused"),
                fresh: false,
            })
            .unwrap(),
        );
        instance.managed_xats = Some(identity);
        let command = command(&instance, "codex", "/tmp").unwrap();
        assert!(command.contains(&thread));
        assert!(command.contains(&profile_name));
        assert!(!command.contains("private-token"));
        assert!(!command.contains("pre-register"));
    }
}
