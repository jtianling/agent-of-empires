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
    pub launch_instance_id: String,
    #[serde(default)]
    pub claim: Option<Claim>,
    #[serde(default)]
    pub pending_restart: Option<String>,
    #[serde(default)]
    pub prepared_previous: Option<Fence>,
    #[serde(default)]
    pub client_config: Option<Value>,
}

impl ManagedXats {
    pub fn new(name: String, team: String) -> Result<Self> {
        let identity = Self {
            name,
            team,
            launch_instance_id: Uuid::new_v4().to_string(),
            claim: None,
            pending_restart: None,
            prepared_previous: None,
            client_config: None,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<()> {
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
        identity.validate()?;
        if identity.claim.is_some() && identity.pending_restart.is_none() {
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
    instance
        .managed_xats
        .as_mut()
        .and_then(|x| x.claim.as_mut())
        .context("missing xats claim")?
        .runtime = Some(runtime);
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
}
