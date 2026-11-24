use anyhow::Result;

use super::{
    client::MissingEndpoint,
    pending::{self, Pending},
};

pub(super) fn resume_legacy_switch(
    profile: &str,
    pending: &Pending,
    error: &anyhow::Error,
) -> Result<bool> {
    let missing_prepare = error
        .downcast_ref::<MissingEndpoint>()
        .is_some_and(|missing| missing.operation == "prepare");
    let original_pair = matches!(
        (
            pending.source_config.tool.as_str(),
            pending.target_tool.as_str()
        ),
        ("claude", "codex") | ("codex", "claude")
    );
    if !missing_prepare || !original_pair || !pending.is_unprepared() {
        return Ok(false);
    }
    pending::discard_unprepared(profile, pending)?;
    tracing::warn!(
        "xats handoff endpoint is unavailable; using the existing Claude/Codex \
        identity-key recovery path for pane {}",
        pending.slot
    );
    Ok(true)
}
