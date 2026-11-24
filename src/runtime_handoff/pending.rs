use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

use super::types::{agent_type, Response, Runtime};
use crate::session::PaneConfig;

const MAX_BYTES: u64 = 64 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pending {
    pub request_id: String,
    pub instance_id: String,
    pub slot: i64,
    pub identity_key: String,
    pub name: String,
    pub team: String,
    pub source_config: PaneConfig,
    pub source_native: String,
    pub source_pane: String,
    pub source_launch: Option<String>,
    pub target_tool: String,
    pub codex_endpoint: Option<String>,
    pub response: Option<Response>,
    pub target_runtime: Option<Runtime>,
    #[serde(default)]
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub controlled_identity: bool,
    #[serde(default)]
    pub prepare_rejected: bool,
}

impl Pending {
    pub fn is_unprepared(&self) -> bool {
        self.response.is_none()
            && self.target_runtime.is_none()
            && self.attempt_id.is_none()
            && !self.controlled_identity
    }
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.request_id).context("Invalid handoff request ID")?;
        if let Some(attempt) = &self.attempt_id {
            uuid::Uuid::parse_str(attempt).context("Invalid handoff launch attempt")?;
        }
        validate_seat(&self.instance_id, self.slot)?;
        crate::xats_control::validate_identity_key(&self.identity_key)?;
        self.source_config.validate()?;
        agent_type(&self.source_config.tool)?;
        agent_type(&self.target_tool)?;
        ensure!(
            !self.name.trim().is_empty() && !self.team.trim().is_empty(),
            "Invalid pending handoff identity"
        );
        if let Some(response) = &self.response {
            self.validate_response(response)?;
        }
        if let Some(runtime) = &self.target_runtime {
            runtime.validate()?;
        }
        Ok(())
    }

    pub fn validate_response(&self, response: &Response) -> Result<()> {
        response.validate()?;
        ensure!(
            response.source.name == self.name
                && response.source.team == self.team
                && response.source.agent_type.as_deref()
                    == Some(agent_type(&self.source_config.tool)?)
                && response.target_agent_type == agent_type(&self.target_tool)?,
            "xats handoff response describes another identity or target"
        );
        if let Some(previous) = &self.response {
            ensure!(
                previous.fence == response.fence
                    && previous.source.agent_id == response.source.agent_id,
                "xats handoff fence changed"
            );
        }
        if let Some(current) = &response.current {
            ensure!(
                current.agent_id == response.source.agent_id
                    && current.name == self.name
                    && current.team == self.team,
                "xats handoff changed the identity"
            );
        }
        Ok(())
    }
}

fn validate_seat(id: &str, slot: i64) -> Result<()> {
    ensure!(
        id.len() == 16
            && id.bytes().all(|byte| byte.is_ascii_hexdigit())
            && (0..=crate::db::MAX_SLOT).contains(&slot),
        "Invalid handoff pane identity"
    );
    Ok(())
}

fn path(profile: &str, id: &str, slot: i64) -> Result<PathBuf> {
    crate::session::tui_instances::validate_profile(profile)?;
    validate_seat(id, slot)?;
    Ok(crate::session::get_profile_dir(profile)?
        .join("runtime-handoffs")
        .join(format!("{id}.{slot}.json")))
}

pub(crate) fn discard_unprepared(profile: &str, pending: &Pending) -> Result<()> {
    ensure!(
        pending.is_unprepared(),
        "Cannot discard an accepted runtime handoff"
    );
    let current = load(profile, &pending.instance_id, pending.slot)?
        .context("Pending handoff disappeared before cleanup")?;
    ensure!(
        current.request_id == pending.request_id && current.is_unprepared(),
        "Pending handoff changed before cleanup"
    );
    fs::remove_file(path(profile, &pending.instance_id, pending.slot)?)
        .context("Removing unaccepted runtime handoff")
}

pub(crate) fn load(profile: &str, id: &str, slot: i64) -> Result<Option<Pending>> {
    let path = path(profile, id, slot)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Inspecting pending runtime handoff"),
    };
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_BYTES,
        "Invalid pending handoff file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "Pending handoff file is not private"
        );
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Pending handoff file exceeds size limit"
    );
    let pending: Pending =
        serde_json::from_slice(&bytes).context("Invalid pending handoff file")?;
    pending.validate()?;
    ensure!(
        pending.instance_id == id && pending.slot == slot,
        "Pending handoff seat mismatch"
    );
    Ok(Some(pending))
}

pub(crate) fn save(profile: &str, pending: &Pending) -> Result<()> {
    pending.validate()?;
    let path = path(profile, &pending.instance_id, pending.slot)?;
    let parent = path.parent().context("Missing handoff directory")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec(pending)?)?;
        file.sync_all()?;
        fs::rename(&temp, &path)?;
        Ok(())
    })();
    if result.is_err() {
        if let Err(cleanup) = fs::remove_file(&temp) {
            if cleanup.kind() != std::io::ErrorKind::NotFound {
                return Err(anyhow::anyhow!(
                    "{:#}; handoff temporary cleanup failed: {cleanup}",
                    result.err().context("Missing handoff write error")?
                ));
            }
        }
    }
    result
}
