//! Migration v005: Remove the obsolete [app_state] dynamic_tab_title field.
//!
//! AoE no longer manages the TUI terminal title, so this config key has no
//! effect and should be removed from persisted config files.

use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, info};

pub fn run() -> Result<()> {
    let app_dir = crate::session::get_app_dir()?;
    let global_config = app_dir.join("config.toml");
    migrate_config_file(&global_config)
}

fn migrate_config_file(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        debug!("Config file {} does not exist, skipping", path.display());
        return Ok(());
    }

    let content = fs::read_to_string(path)?;
    let mut doc: toml::Table = match content.parse() {
        Ok(table) => table,
        Err(e) => {
            debug!("Failed to parse {}: {}, skipping", path.display(), e);
            return Ok(());
        }
    };

    let should_remove_app_state = {
        let Some(app_state) = doc.get_mut("app_state").and_then(|v| v.as_table_mut()) else {
            debug!("No [app_state] table in {}, skipping", path.display());
            return Ok(());
        };

        if app_state.remove("dynamic_tab_title").is_none() {
            debug!(
                "No [app_state] dynamic_tab_title in {}, skipping",
                path.display()
            );
            return Ok(());
        }

        info!(
            "Removing [app_state] dynamic_tab_title from {}",
            path.display()
        );
        app_state.is_empty()
    };

    if should_remove_app_state {
        doc.remove("app_state");
    }

    let new_content = toml::to_string_pretty(&doc)?;
    fs::write(path, new_content)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_removes_dynamic_tab_title() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");

        let content = r#"
[app_state]
has_seen_welcome = true
dynamic_tab_title = false
"#;
        fs::write(&config_path, content).unwrap();

        migrate_config_file(&config_path).unwrap();

        let result: toml::Table = fs::read_to_string(&config_path).unwrap().parse().unwrap();
        assert_eq!(
            result["app_state"]["has_seen_welcome"].as_bool(),
            Some(true)
        );
        assert!(result["app_state"]
            .as_table()
            .unwrap()
            .get("dynamic_tab_title")
            .is_none());
    }

    #[test]
    fn test_migrate_removes_empty_app_state_table() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");

        let content = r#"
[app_state]
dynamic_tab_title = true
"#;
        fs::write(&config_path, content).unwrap();

        migrate_config_file(&config_path).unwrap();

        let result: toml::Table = fs::read_to_string(&config_path).unwrap().parse().unwrap();
        assert!(result.get("app_state").is_none());
    }

    #[test]
    fn test_migrate_is_noop_when_field_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");

        let content = r#"
[app_state]
has_seen_welcome = true
"#;
        fs::write(&config_path, content).unwrap();

        migrate_config_file(&config_path).unwrap();

        let result: toml::Table = fs::read_to_string(&config_path).unwrap().parse().unwrap();
        assert_eq!(
            result["app_state"]["has_seen_welcome"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_migrate_nonexistent_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("nonexistent.toml");

        migrate_config_file(&config_path).unwrap();
    }
}
