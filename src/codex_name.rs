use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::codex_permissions::request;
use crate::codex_xats::{self, BindingRequest};
use crate::xats_control::{self, ControlPlane};
use crate::xats_identity::KeyHolder;

const STARTUP_WAIT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq, Eq)]
struct NameTarget {
    thread_id: String,
    name: String,
}

pub(crate) fn session_name(name: &str, team: &str) -> Result<String> {
    for value in [name, team] {
        ensure!(
            !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control),
            "Invalid Codex xats session name"
        );
    }
    Ok(format!("{name}@{team}"))
}

pub(crate) fn monitor_command(endpoint: &str) -> Result<String> {
    use crate::session::environment::shell_escape;

    let executable = std::env::current_exe().context("Locating Codex name helper")?;
    let executable = executable
        .to_str()
        .context("AoE executable path is not UTF-8")?;
    let log = crate::session::get_app_dir_path()?.join("debug.log");
    let log = log.to_str().context("AoE log path is not UTF-8")?;
    Ok(format!(
        "{} __codex-name --launch-id \"$xats_agent_id\" --endpoint {} \
         </dev/null >/dev/null 2>>{} &",
        shell_escape(executable),
        shell_escape(endpoint),
        shell_escape(log)
    ))
}

pub(crate) async fn run(launch_id: &str, endpoint: &str) -> Result<()> {
    uuid::Uuid::try_parse(launch_id).context("Invalid Codex name launch ID")?;
    let url = url::Url::parse(endpoint).context("Invalid Codex name endpoint")?;
    ensure!(
        matches!(url.scheme(), "ws" | "wss") && url.host_str().is_some(),
        "Codex name endpoint requires ws:// or wss://"
    );
    let key = match std::env::var(xats_control::IDENTITY_KEY_ENV) {
        Ok(key) if !key.is_empty() => key,
        Ok(_) | Err(std::env::VarError::NotPresent) => return Ok(()),
        Err(error) => return Err(error).context("Invalid Codex pane identity key"),
    };
    xats_control::validate_identity_key(&key)?;
    let binding = BindingRequest {
        protocol_version: xats_control::PROTOCOL_VERSION,
        pane_id: std::env::var("TMUX_PANE").context("Missing Codex name pane")?,
        launch_id: launch_id.to_string(),
    };
    codex_xats::validate_request(&binding)?;
    let control = xats_control::discover_control_plane()?;
    let client = xats_control::build_control_client(xats_control::CONTROL_TIMEOUT)?;
    let parent = crate::process::parent_id();
    tokio::time::timeout(STARTUP_WAIT, async {
        loop {
            if parent <= 1 || crate::process::parent_id() != parent {
                return Ok(());
            }
            if let Some(target) = target(&control, &client, &binding, &key).await? {
                return rename(endpoint, &control, &client, &binding, &key, target).await;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .context("Timed out waiting for the Codex launch and registered xats identity")?
}

async fn target(
    control: &ControlPlane,
    client: &reqwest::Client,
    binding: &BindingRequest,
    key: &str,
) -> Result<Option<NameTarget>> {
    match crate::db::codex_capture::current_launch_id_async(&binding.pane_id).await? {
        Some(current) if current == binding.launch_id => {}
        Some(_) => bail!("Codex name helper belongs to a previous launch"),
        None => return Ok(None),
    }
    let Some(thread_id) = codex_xats::lookup_with(control, client, binding).await? else {
        return Ok(None);
    };
    match crate::xats_identity::lookup_with(control, client, key).await {
        KeyHolder::Held {
            name,
            team,
            agent_type,
            active: true,
        } if agent_type.as_deref() == Some("codex") => {
            ensure!(
                crate::db::codex_capture::current_launch_id_async(&binding.pane_id)
                    .await?
                    .as_deref()
                    == Some(binding.launch_id.as_str()),
                "Codex launch changed during session name lookup"
            );
            Ok(Some(NameTarget {
                thread_id,
                name: session_name(&name, &team)?,
            }))
        }
        KeyHolder::NotFound | KeyHolder::Held { .. } => Ok(None),
        KeyHolder::Unsupported => bail!("xats identity lookup is not supported"),
        KeyHolder::Unavailable(error) => bail!("Codex name identity lookup failed: {error}"),
    }
}

async fn rename(
    endpoint: &str,
    control: &ControlPlane,
    client: &reqwest::Client,
    binding: &BindingRequest,
    key: &str,
    expected: NameTarget,
) -> Result<()> {
    let (mut socket, _) = connect_async(endpoint)
        .await
        .context("Connecting to name Codex")?;
    request(
        &mut socket,
        1,
        "initialize",
        json!({"clientInfo": {"name": "aoe-name", "version": env!("CARGO_PKG_VERSION")}}),
    )
    .await
    .context("Initializing the Codex name client")?;
    socket
        .send(Message::Text(
            json!({"method": "initialized"}).to_string().into(),
        ))
        .await?;
    ensure!(
        target(control, client, binding, key).await?.as_ref() == Some(&expected),
        "Codex launch or xats identity changed before naming"
    );
    request(
        &mut socket,
        2,
        "thread/name/set",
        json!({"threadId": expected.thread_id, "name": expected.name}),
    )
    .await
    .context("Renaming the registered Codex session")?;
    verify_name(&mut socket, &expected).await?;
    socket
        .close(None)
        .await
        .context("Closing Codex name client")
}

async fn verify_name(
    socket: &mut crate::codex_permissions::Socket,
    target: &NameTarget,
) -> Result<()> {
    #[derive(Deserialize)]
    struct Response {
        thread: Thread,
    }
    #[derive(Deserialize)]
    struct Thread {
        id: String,
        name: String,
    }
    let response = request(
        socket,
        3,
        "thread/read",
        json!({"threadId": target.thread_id}),
    )
    .await?;
    let response: Response =
        serde_json::from_value(response).context("Invalid named Codex thread")?;
    ensure!(
        response.thread.id == target.thread_id && response.thread.name == target.name,
        "Codex did not persist the requested session name"
    );
    Ok(())
}
