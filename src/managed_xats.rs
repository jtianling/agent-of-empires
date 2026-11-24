use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::session::Instance;

mod transport;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fence {
    pub claim_id: String,
    pub instance_id: String,
    pub generation: u64,
}

impl Fence {
    fn validate(&self) -> Result<()> {
        Uuid::try_parse(&self.claim_id).context("invalid managed claim ID")?;
        Uuid::try_parse(&self.instance_id).context("invalid managed instance ID")?;
        ensure!(
            self.generation > 0 && self.generation <= 9_007_199_254_740_991,
            "invalid managed generation"
        );
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub fence: Fence,
    mcp_token: String,
    pub runtime: Option<Value>,
}

impl std::fmt::Debug for Claim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Claim").field("fence", &self.fence).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedXats {
    pub name: String,
    pub team: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub launch_instance_id: String,
    #[serde(default)]
    pub claim: Option<Claim>,
    #[serde(default)]
    pub pending_restart: Option<String>,
    #[serde(default)]
    pub prepared_previous: Option<Fence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepared_agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retiring_codex: Option<RetiringCodex>,
    #[serde(default)]
    pub client_config: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetiringCodex {
    pub client_config: Value,
    pub launch_instance_id: String,
    pub fence: Option<Fence>,
}

impl ManagedXats {
    pub fn new(name: String, team: String) -> Result<Self> {
        let identity = Self {
            name,
            team,
            model: None,
            effort: None,
            launch_instance_id: Uuid::new_v4().to_string(),
            claim: None,
            pending_restart: None,
            prepared_previous: None,
            previous_agent_type: None,
            prepared_agent_type: None,
            retiring_codex: None,
            client_config: None,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(previous) = &self.previous_agent_type {
            ensure!(
                self.claim.is_some()
                    && matches!(previous.as_str(), "codex" | "claude-code" | "opencode"),
                "Invalid previous managed agent type"
            );
        }
        if let Some(effort) = &self.effort {
            if self.model.is_none()
                || effort.is_empty()
                || effort.len() > 32
                || !effort
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                bail!("invalid managed reasoning effort");
            }
        }
        if let Some(model) = &self.model {
            if model.is_empty()
                || model.len() > 128
                || !model.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/')
                })
            {
                bail!("invalid managed model ID");
            }
        }
        for (field, value) in [("name", &self.name), ("team", &self.team)] {
            if value.trim().is_empty()
                || value.trim() != value
                || value.chars().any(char::is_control)
            {
                bail!("invalid xats {field}");
            }
        }
        Uuid::try_parse(&self.launch_instance_id).context("invalid launch instance ID")?;
        if let Some(claim) = &self.claim {
            claim.fence.validate()?;
        }
        if let Some(previous) = &self.prepared_previous {
            previous.validate()?;
        }
        if let Some(pending) = &self.pending_restart {
            Uuid::try_parse(pending).context("invalid pending restart ID")?;
        }
        Ok(())
    }
}

pub fn stage_restart(instance: &mut Instance) -> Result<()> {
    if let Some(identity) = &mut instance.managed_xats {
        let target_type = agent_type(&instance.tool)?.to_string();
        identity.validate()?;
        if identity.claim.is_some() && identity.pending_restart.is_none() {
            let source_type = identity
                .prepared_agent_type
                .clone()
                .or_else(|| {
                    identity
                        .claim
                        .as_ref()
                        .and_then(|claim| claim.runtime.as_ref())
                        .and_then(|runtime| runtime.get("agent_type"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .or_else(|| identity.previous_agent_type.clone())
                .unwrap_or_else(|| target_type.clone());
            identity.previous_agent_type = (source_type != target_type).then_some(source_type);
            identity.pending_restart = Some(Uuid::new_v4().to_string());
        }
    }
    Ok(())
}

pub fn prepare_restart(instance: &mut Instance) -> Result<()> {
    prepare(instance)
}

pub fn prepare(instance: &mut Instance) -> Result<()> {
    let Some(identity) = &instance.managed_xats else {
        return Ok(());
    };
    identity.validate()?;
    let key = identity_key(instance)?;
    let instance_id = identity
        .pending_restart
        .as_ref()
        .unwrap_or(&identity.launch_instance_id);
    let mut request = json!({
        "protocol_version": 1, "identity_key": key,
        "name": identity.name, "team": identity.team,
        "agent_type": agent_type(&instance.tool)?, "instance_id": instance_id
    });
    let previous = if identity.pending_restart.is_some() {
        Some(
            identity
                .claim
                .as_ref()
                .context("restart has no previous claim")?
                .fence
                .clone(),
        )
    } else {
        identity.prepared_previous.clone()
    };
    if let Some(previous) = &previous {
        request["previous"] = json!(previous);
        if let Some(previous_type) = &identity.previous_agent_type {
            request["previous_agent_type"] = json!(previous_type);
        }
    }
    let response = transport::call("prepare", request, key, None)?;
    ensure!(
        response.fence.instance_id == *instance_id,
        "xats prepare returned another instance"
    );
    if let Some(previous) = &identity.claim {
        ensure!(
            response.fence.claim_id == previous.fence.claim_id,
            "xats prepare changed the claim identity"
        );
        let expected = previous.fence.generation + u64::from(identity.pending_restart.is_some());
        ensure!(
            response.fence.generation == expected,
            "xats prepare returned an unexpected generation"
        );
    }
    let token = response
        .mcp_token
        .context("xats prepare omitted MCP token")?;
    ensure!(
        !token.is_empty() && !token.chars().any(char::is_control),
        "xats prepare returned invalid MCP token"
    );
    let identity = instance
        .managed_xats
        .as_mut()
        .context("missing xats identity")?;
    identity.launch_instance_id = response.fence.instance_id.clone();
    identity.pending_restart = None;
    identity.prepared_previous = previous;
    identity.prepared_agent_type = Some(agent_type(&instance.tool)?.to_string());
    identity.claim = Some(Claim {
        fence: response.fence,
        mcp_token: token,
        runtime: response.runtime,
    });
    Ok(())
}

pub fn bind(instance: &mut Instance, runtime: Value) -> Result<()> {
    let runtime = transport::canonical_runtime(&runtime)?;
    let key = identity_key(instance)?;
    let claim = claim(instance)?;
    ensure!(
        runtime["agent_type"] == agent_type(&instance.tool)?,
        "xats runtime type differs from session tool"
    );
    let mut request = control_request(instance)?;
    request["runtime"] = runtime.clone();
    let response = transport::call("bind", request, key, Some(&claim.mcp_token))?;
    ensure!(
        response.fence == claim.fence && response.runtime.as_ref() == Some(&runtime),
        "xats bind returned mismatched runtime or fence"
    );
    let identity = instance
        .managed_xats
        .as_mut()
        .context("missing xats identity")?;
    identity
        .claim
        .as_mut()
        .context("missing xats claim")?
        .runtime = Some(runtime);
    identity.previous_agent_type = None;
    Ok(())
}

pub fn status(instance: &Instance) -> Result<Value> {
    let Some(identity) = &instance.managed_xats else {
        return Ok(json!({"state": "disabled", "runtime_ready": false}));
    };
    let Some(claim) = &identity.claim else {
        return Ok(json!({"state": "unprepared", "name": identity.name,
            "team": identity.team, "runtime_ready": false}));
    };
    let mut request = control_request(instance)?;
    if let Some(runtime) = &claim.runtime {
        request["runtime"] = runtime.clone();
    }
    let response = transport::call(
        "status",
        request,
        identity_key(instance)?,
        Some(&claim.mcp_token),
    )?;
    ensure!(
        response.fence == claim.fence,
        "xats status returned another fence"
    );
    if let Some(runtime) = &claim.runtime {
        let runtime = transport::canonical_runtime(runtime)?;
        ensure!(
            response.runtime.as_ref() == Some(&runtime) || response.state == "released",
            "xats status returned another runtime"
        );
    }
    Ok(
        json!({"state": response.state, "name": identity.name, "team": identity.team,
        "runtime_ready": response.state == "ready", "agent_id": response.agent_id,
        "runtime": response.runtime}),
    )
}

pub fn status_report(instance: &Instance) -> Value {
    match status(instance) {
        Ok(status) => status,
        Err(error) => {
            let detail = format!("{error:#}");
            tracing::warn!("Managed xats status unavailable: {detail}");
            json!({"state": "unavailable", "runtime_ready": false, "error": detail})
        }
    }
}

pub fn prepare_release(instance: &mut Instance) -> Result<()> {
    let Some(identity) = &instance.managed_xats else {
        return Ok(());
    };
    if identity.claim.is_some() && identity.pending_restart.is_none() {
        return Ok(());
    }
    let never_claimed = identity.claim.is_none() && identity.pending_restart.is_none();
    if let Err(error) = prepare(instance) {
        if never_claimed
            && error
                .downcast_ref::<transport::DomainFailure>()
                .is_some_and(|error| error.code == "identity_conflict")
        {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

pub fn release(instance: &Instance) -> Result<()> {
    let Some(identity) = &instance.managed_xats else {
        return Ok(());
    };
    ensure!(
        identity.pending_restart.is_none(),
        "Persist the recovered managed fence before releasing it"
    );
    if identity.claim.is_none() {
        return Ok(());
    }
    let claim = claim(instance)?;
    let response = transport::call(
        "release",
        control_request(instance)?,
        identity_key(instance)?,
        Some(&claim.mcp_token),
    )?;
    ensure!(
        response.fence == claim.fence && response.state == "released",
        "xats release returned mismatched fence or state"
    );
    Ok(())
}

pub fn mcp_token(instance: &Instance) -> Result<&str> {
    Ok(&claim(instance)?.mcp_token)
}

pub fn runtime(instance: &Instance) -> Option<&Value> {
    instance
        .managed_xats
        .as_ref()?
        .claim
        .as_ref()?
        .runtime
        .as_ref()
}

fn claim(instance: &Instance) -> Result<&Claim> {
    instance
        .managed_xats
        .as_ref()
        .and_then(|x| x.claim.as_ref())
        .context("managed xats identity is not prepared")
}

fn identity_key(instance: &Instance) -> Result<&str> {
    let key = instance
        .xats_identity_key
        .as_deref()
        .context("missing xats identity key")?;
    crate::xats_control::validate_identity_key(key)?;
    Ok(key)
}

fn control_request(instance: &Instance) -> Result<Value> {
    Ok(
        json!({"protocol_version": 1, "identity_key": identity_key(instance)?,
        "fence": claim(instance)?.fence}),
    )
}

fn agent_type(tool: &str) -> Result<&str> {
    match tool {
        "claude" => Ok("claude-code"),
        "codex" => Ok("codex"),
        "opencode" => Ok("opencode"),
        "kimi" => bail!("Managed xats for Kimi requires session-scoped MCP token support"),
        _ => bail!("unsupported managed xats tool: {tool}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_restart_survives_serialization_and_reuses_request_id() {
        let mut instance = Instance::new("worker", "/tmp");
        let mut identity = ManagedXats::new("worker".into(), "team".into()).unwrap();
        identity.claim = Some(Claim {
            fence: Fence {
                claim_id: Uuid::new_v4().to_string(),
                instance_id: identity.launch_instance_id.clone(),
                generation: 1,
            },
            mcp_token: "private-test-token".into(),
            runtime: None,
        });
        instance.managed_xats = Some(identity);
        stage_restart(&mut instance).unwrap();
        let serialized = serde_json::to_string(&instance).unwrap();
        let mut restored: Instance = serde_json::from_str(&serialized).unwrap();
        stage_restart(&mut restored).unwrap();
        assert_eq!(instance.managed_xats, restored.managed_xats);
        let identity = restored.managed_xats.unwrap();
        assert_ne!(
            identity.pending_restart.as_deref(),
            Some(identity.launch_instance_id.as_str())
        );
        assert!(!format!("{identity:?}").contains("private-test-token"));
    }

    #[test]
    fn rejects_invalid_identity_and_fence() {
        assert!(ManagedXats::new(" ".into(), "team".into()).is_err());
        assert!(ManagedXats::new("worker".into(), "team\n".into()).is_err());
        let mut identity = ManagedXats::new("worker".into(), "team".into()).unwrap();
        identity.prepared_previous = Some(Fence {
            claim_id: "wrong".into(),
            instance_id: Uuid::new_v4().to_string(),
            generation: 1,
        });
        assert!(identity.validate().is_err());
    }

    #[test]
    fn switching_a_claim_keeps_its_source_type_and_fence_across_restart() {
        let mut instance = Instance::new("worker", "/tmp/worktree");
        instance.tool = "codex".into();
        instance.sync_primary_pane_from_legacy();
        let mut identity = ManagedXats::new("worker".into(), "team".into()).unwrap();
        let source = Fence {
            claim_id: Uuid::new_v4().to_string(),
            instance_id: identity.launch_instance_id.clone(),
            generation: 4,
        };
        identity.claim = Some(Claim {
            fence: source.clone(),
            mcp_token: "private-test-token".into(),
            runtime: Some(json!({"agent_type": "codex"})),
        });
        instance.managed_xats = Some(identity);
        instance
            .switch_agent("opencode", &crate::session::Config::default())
            .unwrap();
        stage_restart(&mut instance).unwrap();
        let restored: Instance =
            serde_json::from_str(&serde_json::to_string(&instance).unwrap()).unwrap();
        let identity = restored.managed_xats.unwrap();
        identity.validate().unwrap();
        assert_eq!(identity.previous_agent_type.as_deref(), Some("codex"));
        assert_eq!(identity.claim.as_ref().unwrap().fence, source);
        assert_ne!(
            identity.pending_restart.as_deref(),
            Some(source.instance_id.as_str())
        );
        assert_eq!((&*identity.name, &*identity.team), ("worker", "team"));
    }

    #[test]
    fn a_prepared_new_type_does_not_reuse_the_old_source_on_its_next_restart() {
        let mut instance = Instance::new("worker", "/tmp/worktree");
        instance.tool = "opencode".into();
        instance.sync_primary_pane_from_legacy();
        let mut identity = ManagedXats::new("worker".into(), "team".into()).unwrap();
        identity.claim = Some(Claim {
            fence: Fence {
                claim_id: Uuid::new_v4().to_string(),
                instance_id: identity.launch_instance_id.clone(),
                generation: 5,
            },
            mcp_token: "private-test-token".into(),
            runtime: None,
        });
        identity.prepared_agent_type = Some("opencode".into());
        identity.previous_agent_type = Some("codex".into());
        instance.managed_xats = Some(identity);
        stage_restart(&mut instance).unwrap();
        let identity = instance.managed_xats.unwrap();
        assert!(identity.previous_agent_type.is_none());
        assert_eq!(identity.prepared_agent_type.as_deref(), Some("opencode"));
        assert!(identity.pending_restart.is_some());
    }
}
