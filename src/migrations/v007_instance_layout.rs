use anyhow::Result;
use tracing::info;

pub fn run() -> Result<()> {
    let profile = std::env::var("AGENT_OF_EMPIRES_PROFILE")
        .unwrap_or_else(|_| crate::session::DEFAULT_PROFILE.to_string());
    info!("Adding layout snapshots for profile '{}'", profile);
    crate::db::create_schema_for_profile(&profile)
}
