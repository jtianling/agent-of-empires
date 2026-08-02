use anyhow::Result;
use tracing::info;

pub fn run() -> Result<()> {
    let active_profile = std::env::var("AGENT_OF_EMPIRES_PROFILE")
        .unwrap_or_else(|_| crate::session::DEFAULT_PROFILE.to_string());
    let mut profiles = crate::session::list_profiles()?;
    if !profiles.contains(&active_profile) {
        profiles.push(active_profile);
    }
    profiles.sort();
    profiles.dedup();

    for profile in profiles {
        info!("Migrating pane launch config for profile '{}'", profile);
        crate::db::create_schema_for_profile(&profile)?;
        let storage = crate::session::Storage::new(&profile)?;
        let instances = storage.load()?;
        let store = crate::db::Store::open_with_schema(&profile)?;
        store.migrate_legacy_pane_configs(&instances)?;
    }
    Ok(())
}
