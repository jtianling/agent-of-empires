//! Create the SQLite agent-session store (`aoe.db`) for the active profile.
//!
//! The store is profile-scoped (one `aoe.db` per profile directory). This
//! migration creates and applies the schema for the currently active profile.
//! Other profiles get their schema lazily via `db::Store::open_with_schema`
//! the first time they are touched (capture subcommand or reconciler), so no
//! global sweep is needed here.

use anyhow::Result;
use tracing::info;

pub fn run() -> Result<()> {
    let profile = std::env::var("AGENT_OF_EMPIRES_PROFILE")
        .unwrap_or_else(|_| crate::session::DEFAULT_PROFILE.to_string());
    info!("Creating agent-session store for profile '{}'", profile);
    crate::db::create_schema_for_profile(&profile)?;
    Ok(())
}
