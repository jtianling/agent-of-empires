use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fence {
    pub handoff_id: String,
    pub register_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "agent_type", deny_unknown_fields)]
pub(crate) enum Runtime {
    #[serde(rename = "codex")]
    Codex { thread_id: String, ws_url: String },
    #[serde(rename = "opencode")]
    OpenCode {
        base_url: String,
        session_id: String,
    },
    #[serde(rename = "claude-code")]
    Claude {
        ui_pid: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_session_id: Option<String>,
    },
}

impl Runtime {
    pub fn kind(&self) -> &str {
        match self {
            Self::Codex { .. } => "codex",
            Self::OpenCode { .. } => "opencode",
            Self::Claude { .. } => "claude-code",
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Codex { thread_id, ws_url } => {
                uuid::Uuid::parse_str(thread_id).context("Invalid handoff thread ID")?;
                validate_url(ws_url, &["ws", "wss"])?;
            }
            Self::OpenCode {
                base_url,
                session_id,
            } => {
                crate::opencode_runtime::validate_session_id(session_id)?;
                validate_url(base_url, &["http", "https"])?;
            }
            Self::Claude {
                ui_pid,
                channel_session_id,
            } => {
                ensure!(*ui_pid > 1, "Invalid handoff Claude PID");
                if let Some(channel) = channel_session_id {
                    ensure!(
                        !channel.is_empty() && !channel.chars().any(char::is_control),
                        "Invalid handoff Claude channel"
                    );
                }
            }
        }
        Ok(())
    }
}

fn validate_url(value: &str, schemes: &[&str]) -> Result<()> {
    let url = url::Url::parse(value).context("Invalid handoff runtime URL")?;
    ensure!(
        schemes.contains(&url.scheme())
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Invalid handoff runtime URL"
    );
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub agent_id: String,
    pub name: String,
    pub team: String,
    pub agent_type: Option<String>,
    pub register_generation: u64,
    pub runtime_generation: Option<i64>,
    pub runtime: Option<Runtime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum State {
    Prepared,
    Committed,
    Aborted,
    Stale,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Response {
    pub ok: bool,
    pub protocol_version: u32,
    pub state: State,
    pub fence: Fence,
    pub source: Identity,
    pub current: Option<Identity>,
    pub target_agent_type: String,
    pub recovery_prompt: Option<String>,
}

impl Response {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.ok && self.protocol_version == 1 && self.recovery_prompt.is_none(),
            "Invalid xats handoff response"
        );
        uuid::Uuid::parse_str(&self.fence.handoff_id).context("Invalid xats handoff ID")?;
        ensure!(
            self.source.register_generation == self.fence.register_generation,
            "xats handoff source generation does not match its fence"
        );
        ensure!(
            self.fence.register_generation <= crate::db::MAX_XATS_RUNTIME_GENERATION as u64,
            "Invalid xats handoff generation"
        );
        for identity in std::iter::once(&self.source).chain(self.current.iter()) {
            uuid::Uuid::parse_str(&identity.agent_id).context("Invalid handoff identity")?;
            ensure!(
                !identity.name.trim().is_empty() && !identity.team.trim().is_empty(),
                "Invalid handoff name or team"
            );
            ensure!(
                identity.register_generation <= crate::db::MAX_XATS_RUNTIME_GENERATION as u64,
                "Invalid handoff register generation"
            );
            if let Some(generation) = identity.runtime_generation {
                ensure!(
                    (0..=crate::db::MAX_XATS_RUNTIME_GENERATION).contains(&generation),
                    "Invalid handoff runtime generation"
                );
            }
            if let Some(runtime) = &identity.runtime {
                runtime.validate()?;
                ensure!(
                    identity.agent_type.as_deref() == Some(runtime.kind()),
                    "xats handoff runtime type does not match its identity"
                );
            }
        }
        Ok(())
    }
}

pub(crate) fn agent_type(tool: &str) -> Result<&str> {
    match tool {
        "codex" | "opencode" => Ok(tool),
        "claude" => Ok("claude-code"),
        _ => anyhow::bail!("Unsupported handoff agent: {tool}"),
    }
}
