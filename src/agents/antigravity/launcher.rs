use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, ensure, Context, Result};

use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::agentapi::{self, AGENTAPI_FLAG};
use crate::agents::executable;
use crate::session::environment::shell_escape;
use crate::xats_control::{self, ControlPlane, CONTROL_TIMEOUT};
use crate::xats_identity::{lookup_with, KeyHolder};

pub(crate) const EXPECTED_AGENT_ENV: &str = "AOE_ANTIGRAVITY_AGENT_ID";

pub(crate) const DEFER_FLAG: &str = "--defer-registration";

pub(crate) struct ControlledLauncher {
    prefix: String,
    agy_extra_args: String,
    pub(crate) deferred: bool,
}

impl ControlledLauncher {
    pub(crate) fn prepare(key: &str, minted: bool, binary: &str, cwd: &str) -> Result<Self> {
        let uuid = Uuid::parse_str(key).context("Invalid Antigravity identity key")?;
        ensure!(
            uuid.get_version_num() == 4
                && uuid.get_variant() == uuid::Variant::RFC4122
                && uuid.to_string() == key,
            "Expected a canonical random Antigravity identity key"
        );
        let identity = if minted { None } else { lookup_identity(key)? };
        let launcher = executable::resolve("xats-agy").context("Install xats-agy on PATH")?;
        let agy_dir = agy_directory(binary)?;
        crate::hooks::antigravity_xats::check_named_mcp(cwd)?;
        let launcher_arg = shell_escape(launcher.to_str().context("Invalid xats-agy path")?);
        let (command, agentapi) = match &identity {
            Some(agent) => {
                check_launcher(&launcher, &agy_dir)?;
                crate::antigravity_mcp::check_named_support(key)?;
                let agentapi = agentapi::probe_agentapi_channel(&launcher);
                let mode_flag = if agentapi {
                    format!(" {AGENTAPI_FLAG}")
                } else {
                    String::new()
                };
                (
                    format!(
                        "{EXPECTED_AGENT_ENV}={} {launcher_arg}{mode_flag} {} {} --",
                        agent.id,
                        label_arg("--name", &agent.name),
                        label_arg("--team", &agent.team)
                    ),
                    agentapi,
                )
            }
            None => {
                let caps = crate::antigravity_mcp::probe_capabilities(&launcher)?;
                ensure!(
                    caps.defer_registration,
                    "Upgrade xats: expected defer_registration capability 1"
                );
                let agentapi = caps.agentapi_channel && agentapi::channel_bind_available();
                let bridge = crate::antigravity_mcp::require_deferred_bridge()?
                    .canonicalize()
                    .context("Resolving verified xats-agy-mcp")?;
                let helper =
                    std::env::current_exe().context("Locating AoE Antigravity launcher")?;
                let mode_flag = if agentapi {
                    format!(" {AGENTAPI_FLAG}")
                } else {
                    String::new()
                };
                (
                    format!(
                        "AOE_ANTIGRAVITY_SLOT_DIGEST={:x} AOE_ANTIGRAVITY_MCP_BINARY={} \
                         AOE_ANTIGRAVITY_MCP_PATH={} {} __antigravity-launch -- {launcher_arg}{mode_flag} {DEFER_FLAG} --",
                        Sha256::digest(key.as_bytes()),
                        shell_escape(bridge.to_str().context("Invalid xats-agy-mcp path")?),
                        shell_escape(
                            &std::env::var("PATH")
                                .context("Invalid Antigravity MCP search path")?
                        ),
                        shell_escape(helper.to_str().context("Invalid AoE executable path")?)
                    ),
                    agentapi,
                )
            }
        };
        let deferred = identity.is_none();
        let agy_extra_args = if agentapi {
            agentapi::agy_channel_args(!deferred)?
        } else {
            String::new()
        };
        Ok(Self {
            prefix: format!("PATH={}:\"$PATH\" {command}", shell_escape(&agy_dir)),
            agy_extra_args,
            deferred,
        })
    }

    pub(crate) fn prefix(&self) -> &str {
        &self.prefix
    }

    pub(crate) fn agy_extra_args(&self) -> &str {
        &self.agy_extra_args
    }
}

fn lookup_identity(key: &str) -> Result<Option<RegisteredAgent>> {
    let control = xats_control::discover_control_plane()?;
    let key = key.to_owned();
    xats_control::block_on_control("agy-slot-lookup", move || {
        Box::pin(async move {
            let client = Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(CONTROL_TIMEOUT)
                .build()?;
            let result = registered_agent(&control, &client, &key).await;
            xats_control::redact_secrets(result, &key, &control)
        })
    })
}

pub(crate) struct RegisteredAgent {
    pub(crate) id: Uuid,
    pub(crate) team: String,
    pub(crate) name: String,
}

pub(crate) async fn registered_agent(
    control: &ControlPlane,
    client: &Client,
    key: &str,
) -> Result<Option<RegisteredAgent>> {
    let (name, team, kind) = match lookup_with(control, client, key).await {
        KeyHolder::NotFound => return Ok(None),
        KeyHolder::Held {
            name,
            team,
            agent_type,
            ..
        } => (name, team, agent_type),
        KeyHolder::Unsupported => {
            bail!("Upgrade the local xats daemon: identity-key lookup is unsupported")
        }
        KeyHolder::Unavailable(reason) => bail!("Antigravity identity lookup failed: {reason}"),
    };
    let kind = kind
        .as_deref()
        .filter(|kind| matches!(*kind, "antigravity" | "custom"))
        .context("Antigravity slot key belongs to an incompatible xats agent type")?;
    for label in [&name, &team] {
        ensure!(
            !label.is_empty() && label.trim() == label && !label.chars().any(char::is_control),
            "Invalid name or team for xats-agy"
        );
    }
    let mut url = control.base_url.join("/api/agents")?;
    url.query_pairs_mut().append_pair("team", &team);
    let mut request = client.get(url);
    if let Some(token) = &control.token {
        request = request.bearer_auth(token.expose());
    }
    let mut response = request
        .send()
        .await
        .context("Reading xats agent directory")?;
    let status = response.status();
    const LIMIT: usize = 1024 * 1024;
    ensure!(
        response.content_length().unwrap_or(0) <= LIMIT as u64,
        "xats agent directory exceeds size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Reading xats agent directory body")?
    {
        ensure!(
            chunk.len() <= LIMIT - bytes.len(),
            "xats agent directory exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    if status != reqwest::StatusCode::OK {
        #[derive(Deserialize)]
        struct Failure {
            error: String,
        }
        let reason = serde_json::from_slice::<Failure>(&bytes)
            .ok()
            .map(|failure| failure.error)
            .unwrap_or_default();
        bail!("Reading xats agent directory: HTTP {status} {reason}");
    }
    let id = directory_id(&bytes, &team, &name, kind)?;
    Ok(Some(RegisteredAgent { id, team, name }))
}

fn directory_id(bytes: &[u8], team: &str, name: &str, kind: &str) -> Result<Uuid> {
    #[derive(Deserialize)]
    struct Row {
        agent_id: String,
        team: String,
        name: String,
        agent_type: Option<String>,
        agent_type_name: Option<String>,
    }
    #[derive(Deserialize)]
    struct Directory {
        agents: Vec<Row>,
    }

    let directory: Directory =
        serde_json::from_slice(bytes).context("Invalid xats agent directory")?;
    let mut matches = directory
        .agents
        .iter()
        .filter(|row| row.team == team && row.name == name);
    let row = matches
        .next()
        .context("Antigravity identity disappeared from the xats agent directory")?;
    ensure!(
        matches.next().is_none(),
        "Ambiguous Antigravity team/name in the xats agent directory"
    );
    ensure!(
        row.agent_type.as_deref() == Some(kind)
            && (kind == "antigravity"
                || (kind == "custom" && row.agent_type_name.as_deref() == Some("Antigravity"))),
        "xats agent directory identity type does not match the Antigravity slot"
    );
    ensure!(
        crate::hooks::antigravity::valid_id(&row.agent_id),
        "Invalid xats agent directory UUID"
    );
    Ok(Uuid::parse_str(&row.agent_id)?)
}

fn label_arg(option: &str, label: &str) -> String {
    let separator = if label.starts_with('-') { "=" } else { " " };
    format!("{option}{separator}{}", shell_escape(label))
}

fn agy_directory(binary: &str) -> Result<String> {
    let words = shell_words::split(binary)?;
    let [word] = words.as_slice() else {
        bail!("Expected a single Antigravity binary");
    };
    let path = if Path::new(word).is_absolute() {
        PathBuf::from(word)
    } else {
        executable::resolve("agy").context("Cannot find agy on PATH")?
    };
    let dir = path.parent().context("Missing agy directory")?;
    let agy = dir.join("agy");
    ensure!(
        executable::is_executable(&agy),
        "xats-agy requires executable {}",
        agy.display()
    );
    let actual = agy.metadata()?;
    let wanted = path
        .metadata()
        .with_context(|| format!("Checking {}", path.display()))?;
    ensure!(
        actual.dev() == wanted.dev() && actual.ino() == wanted.ino(),
        "xats-agy cannot select the configured binary via {}",
        agy.display()
    );
    let dir = dir.to_str().context("Invalid agy directory")?;
    ensure!(
        !dir.contains([':', '\n', '\r']),
        "Invalid agy PATH directory"
    );
    Ok(dir.to_owned())
}

fn check_launcher(launcher: &Path, dir: &str) -> Result<()> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(PathBuf::from(dir)).chain(std::env::split_paths(&path)),
    )?;
    let mut command = tokio::process::Command::new(launcher);
    command
        .arg("--help")
        .env("PATH", path)
        .env_remove("XATS_IDENTITY_KEY")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    for (key, _) in std::env::vars_os() {
        if key.as_encoded_bytes().starts_with(b"XATS_AGY_") {
            command.env_remove(key);
        }
    }
    xats_control::block_on_control("agy-launcher-check", move || {
        Box::pin(async move {
            let output = tokio::time::timeout(CONTROL_TIMEOUT, command.output())
                .await
                .context("xats-agy --help timed out")?
                .context("Cannot execute xats-agy --help")?;
            ensure!(
                output.status.success(),
                "xats-agy --help failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            Ok(())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn directory_requires_one_matching_antigravity_identity() {
        let id = Uuid::new_v4();
        let row = json!({"agent_id":id.to_string(),"team":"dev","name":"agy",
            "agent_type":"custom","agent_type_name":"Antigravity"});
        let lookup = |rows, kind| {
            directory_id(
                &serde_json::to_vec(&json!({"agents":rows})).unwrap(),
                "dev",
                "agy",
                kind,
            )
        };
        assert_eq!(lookup(vec![row.clone()], "custom").unwrap(), id);
        assert!(lookup(vec![], "custom").is_err());
        let mut other = row.clone();
        other["agent_type_name"] = json!("Other");
        assert!(lookup(vec![other.clone()], "custom").is_err());
        assert!(lookup(vec![row.clone(), other], "custom").is_err());
        let mut native = row.clone();
        native["agent_type"] = json!("antigravity");
        native["agent_type_name"] = json!(null);
        assert!(lookup(vec![native.clone()], "custom").is_err());
        assert_eq!(lookup(vec![native.clone()], "antigravity").unwrap(), id);
        native["agent_id"] = json!(Uuid::nil().to_string());
        assert!(lookup(vec![native], "antigravity").is_err());
    }
}
