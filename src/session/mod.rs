//! Session management module

pub mod builder;
pub mod civilizations;
pub mod config;
mod container_config;
mod environment;
mod groups;
mod instance;
pub mod profile_config;
pub mod repo_config;
mod storage;

pub use crate::sound::{SoundConfig, SoundConfigOverride};
pub use config::{
    get_claude_config_dir, get_update_settings, load_config, save_config, ClaudeConfig, Config,
    ContainerRuntimeName, SandboxConfig, SessionConfig, ThemeConfig, TmuxMouseMode,
    TmuxStatusBarMode, UpdatesConfig, WorktreeConfig,
};
pub use environment::validate_env_entry;
pub use groups::{
    expanded_groups, flatten_tree, flatten_tree_all_profiles, validate_group_path, Group,
    GroupTree, Item,
};
pub(crate) use instance::{extract_resume_token, is_valid_resume_token};
pub use instance::{
    Instance, SandboxInfo, Status, StatusUpdateOptions, TerminalInfo, WorkspaceInfo, WorkspaceRepo,
    WorktreeInfo,
};
pub use profile_config::{
    load_profile_config, merge_configs, resolve_config, save_profile_config,
    validate_check_interval, validate_memory_limit, validate_path_exists, validate_volume_format,
    ClaudeConfigOverride, HooksConfigOverride, ProfileConfig, SandboxConfigOverride,
    SessionConfigOverride, ThemeConfigOverride, TmuxConfigOverride, UpdatesConfigOverride,
    WorktreeConfigOverride,
};
pub use repo_config::{
    check_hook_trust, execute_hooks, execute_hooks_in_container, load_repo_config,
    merge_repo_config, profile_to_repo_config, repo_config_to_profile, resolve_config_with_repo,
    save_repo_config, trust_repo, HookTrustStatus, HooksConfig, RepoConfig,
};
pub use storage::Storage;

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_PROFILE: &str = "default";
const AUTO_PROFILE_PREFIX: &str = "auto-";

pub fn get_app_dir() -> Result<PathBuf> {
    let dir = get_app_dir_path()?;
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

fn get_app_dir_path() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot find config directory"))?
        .join("agent-of-empires");

    #[cfg(not(target_os = "linux"))]
    let dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?
        .join(".agent-of-empires");

    Ok(dir)
}

pub fn get_profile_dir(profile: &str) -> Result<PathBuf> {
    let base = get_app_dir()?;
    let profile_name = if profile.is_empty() {
        DEFAULT_PROFILE
    } else {
        profile
    };
    let dir = base.join("profiles").join(profile_name);
    Ok(dir)
}

/// Like `get_profile_dir`, but creates the directory if it doesn't exist.
pub fn ensure_profile_dir(profile: &str) -> Result<PathBuf> {
    let dir = get_profile_dir(profile)?;
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

pub fn list_profiles() -> Result<Vec<String>> {
    let base = get_app_dir()?;
    let profiles_dir = base.join("profiles");

    if !profiles_dir.exists() {
        return Ok(vec![]);
    }

    let mut profiles = Vec::new();
    for entry in fs::read_dir(&profiles_dir)? {
        let entry = entry?;
        if entry.path().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                profiles.push(name.to_string());
            }
        }
    }
    profiles.sort();
    Ok(profiles)
}

pub fn create_profile(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Profile name cannot be empty");
    }
    if name.contains('/') || name.contains('\\') {
        anyhow::bail!("Profile name cannot contain path separators");
    }
    if name.eq_ignore_ascii_case("all") {
        anyhow::bail!("Profile name 'all' is reserved");
    }

    let profiles = list_profiles()?;
    if profiles.contains(&name.to_string()) {
        anyhow::bail!("Profile '{}' already exists", name);
    }

    ensure_profile_dir(name)?;
    Ok(())
}

pub fn delete_profile(name: &str) -> Result<()> {
    let base = get_app_dir()?;
    let profile_dir = base.join("profiles").join(name);

    if !profile_dir.exists() {
        anyhow::bail!("Profile '{}' does not exist", name);
    }

    if let Ok(storage) = Storage::new(name) {
        if let Ok(instances) = storage.load() {
            for inst in &instances {
                let _ = inst.kill();
                crate::hooks::cleanup_hook_status_dir(&inst.id);
            }
        }
    }

    fs::remove_dir_all(&profile_dir)?;
    Ok(())
}

pub fn rename_profile(old_name: &str, new_name: &str) -> Result<()> {
    if new_name.is_empty() {
        anyhow::bail!("New profile name cannot be empty");
    }
    if new_name.contains('/') || new_name.contains('\\') {
        anyhow::bail!("Profile name cannot contain path separators");
    }

    let base = get_app_dir()?;
    let old_dir = base.join("profiles").join(old_name);
    let new_dir = base.join("profiles").join(new_name);

    if !old_dir.exists() {
        anyhow::bail!("Profile '{}' does not exist", old_name);
    }
    if new_dir.exists() {
        anyhow::bail!("Profile '{}' already exists", new_name);
    }

    fs::rename(&old_dir, &new_dir)?;

    // Update default profile if the renamed profile was the default
    if let Some(config) = load_config()? {
        if config.default_profile == old_name {
            set_default_profile(new_name)?;
        }
    }

    Ok(())
}

pub fn set_default_profile(name: &str) -> Result<()> {
    let mut config = load_config()?.unwrap_or_default();
    config.default_profile = name.to_string();
    save_config(&config)?;
    Ok(())
}

pub fn resolve_profile(explicit: Option<String>) -> String {
    if let Some(p) = explicit {
        return p;
    }

    let cwd = std::env::current_dir().unwrap_or_default();
    let dir_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let sanitized = sanitize_profile_name(dir_name);
    let hash = short_hash(&cwd.to_string_lossy());
    if sanitized.is_empty() {
        format!("{AUTO_PROFILE_PREFIX}{hash}")
    } else {
        format!("{AUTO_PROFILE_PREFIX}{sanitized}-{hash}")
    }
}

fn sanitize_profile_name(name: &str) -> String {
    let lowered = name.to_lowercase();
    let replaced: String = lowered
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let collapsed = replaced
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    collapsed
}

fn short_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    format!("{:02x}{:02x}", result[0], result[1])
}

pub fn register_instance(profile: &str) {
    let Ok(profile_dir) = ensure_profile_dir(profile) else {
        return;
    };
    let instances_dir = profile_dir.join(".instances");
    let _ = fs::create_dir_all(&instances_dir);
    let pid = std::process::id();
    let _ = fs::write(instances_dir.join(pid.to_string()), "");
}

pub fn unregister_instance(profile: &str) {
    let Ok(base) = get_app_dir() else { return };
    let instances_dir = base.join("profiles").join(profile).join(".instances");
    let pid = std::process::id();
    let _ = fs::remove_file(instances_dir.join(pid.to_string()));
}

pub fn cleanup_stale_instances(profile: &str) {
    let Ok(base) = get_app_dir() else { return };
    let instances_dir = base.join("profiles").join(profile).join(".instances");
    if !instances_dir.exists() {
        return;
    }
    let entries = match fs::read_dir(&instances_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if let Ok(pid) = name.parse::<u32>() {
                if !is_process_alive(pid) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
}

pub fn has_other_instances(profile: &str) -> bool {
    let Ok(base) = get_app_dir() else {
        return false;
    };
    let instances_dir = base.join("profiles").join(profile).join(".instances");
    if !instances_dir.exists() {
        return false;
    }
    let my_pid = std::process::id();
    let entries = match fs::read_dir(&instances_dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if let Ok(pid) = name.parse::<u32>() {
                if pid != my_pid && is_process_alive(pid) {
                    return true;
                }
            }
        }
    }
    false
}

fn is_process_alive(pid: u32) -> bool {
    use nix::sys::signal;
    use nix::unistd::Pid;
    signal::kill(Pid::from_raw(pid as i32), None).is_ok()
}

pub fn cleanup_empty_profile(profile: &str) {
    if has_other_instances(profile) {
        return;
    }
    let storage = match Storage::new(profile) {
        Ok(s) => s,
        Err(_) => return,
    };
    let sessions = match storage.load() {
        Ok(s) => s,
        Err(_) => return,
    };
    if sessions.is_empty() {
        let _ = delete_profile(profile);
    }
}

pub fn check_migration_hint(resolved_profile: &str) {
    if resolved_profile == DEFAULT_PROFILE || !resolved_profile.starts_with(AUTO_PROFILE_PREFIX) {
        return;
    }

    let Ok(base) = get_app_dir() else { return };
    let default_dir = base.join("profiles").join(DEFAULT_PROFILE);
    if !default_dir.exists() {
        return;
    }

    let default_has_sessions = Storage::new(DEFAULT_PROFILE)
        .ok()
        .and_then(|s| s.load().ok())
        .map(|sessions| !sessions.is_empty())
        .unwrap_or(false);

    if !default_has_sessions {
        return;
    }

    let resolved_dir = base.join("profiles").join(resolved_profile);
    let resolved_is_empty = if resolved_dir.exists() {
        Storage::new(resolved_profile)
            .ok()
            .and_then(|s| s.load().ok())
            .map(|sessions| sessions.is_empty())
            .unwrap_or(true)
    } else {
        true
    };

    if resolved_is_empty {
        eprintln!(
            "Hint: Your existing sessions are in the 'default' profile. \
             Use `aoe -p default` to access them."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::tempdir;

    fn setup_test_home(temp: &std::path::Path) {
        std::env::set_var("HOME", temp);
        #[cfg(target_os = "linux")]
        std::env::set_var("XDG_CONFIG_HOME", temp.join(".config"));
    }

    #[test]
    fn test_sanitize_profile_name_basic() {
        assert_eq!(sanitize_profile_name("project-a"), "project-a");
        assert_eq!(sanitize_profile_name("MyProject"), "myproject");
        assert_eq!(sanitize_profile_name("hello world"), "hello-world");
    }

    #[test]
    fn test_sanitize_profile_name_special_chars() {
        assert_eq!(sanitize_profile_name("My Project!!"), "my-project");
        assert_eq!(sanitize_profile_name("foo@bar#baz"), "foo-bar-baz");
        assert_eq!(sanitize_profile_name("---leading---"), "leading");
        assert_eq!(sanitize_profile_name("a--b"), "a-b");
    }

    #[test]
    fn test_sanitize_profile_name_unicode() {
        assert_eq!(sanitize_profile_name("项目"), "");
        assert_eq!(sanitize_profile_name("project-项目"), "project");
        assert_eq!(sanitize_profile_name("café"), "caf");
    }

    #[test]
    fn test_sanitize_profile_name_empty() {
        assert_eq!(sanitize_profile_name(""), "");
        assert_eq!(sanitize_profile_name("---"), "");
        assert_eq!(sanitize_profile_name("!!!"), "");
    }

    #[test]
    fn test_short_hash_deterministic() {
        let h1 = short_hash("/home/user/project-a");
        let h2 = short_hash("/home/user/project-a");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 4);
    }

    #[test]
    fn test_short_hash_different_inputs() {
        let h1 = short_hash("/home/user/project-a");
        let h2 = short_hash("/home/user/project-b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_short_hash_hex_format() {
        let h = short_hash("test");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_resolve_profile_explicit() {
        let result = resolve_profile(Some("myprofile".to_string()));
        assert_eq!(result, "myprofile");
    }

    #[test]
    fn test_resolve_profile_directory_based() {
        let result = resolve_profile(None);
        assert!(result.starts_with("auto-"));
        assert!(result.len() > 5);
    }

    #[test]
    #[serial]
    fn test_register_and_unregister_instance() {
        let temp = tempdir().unwrap();
        setup_test_home(temp.path());

        let test_profile = "test-instance-tracking";
        let _ = delete_profile(test_profile);
        create_profile(test_profile).unwrap();

        register_instance(test_profile);

        let base = get_app_dir().unwrap();
        let instances_dir = base.join("profiles").join(test_profile).join(".instances");
        let pid = std::process::id();
        assert!(instances_dir.join(pid.to_string()).exists());

        unregister_instance(test_profile);
        assert!(!instances_dir.join(pid.to_string()).exists());

        let _ = delete_profile(test_profile);
    }

    #[test]
    #[serial]
    fn test_has_other_instances_false_when_alone() {
        let temp = tempdir().unwrap();
        setup_test_home(temp.path());

        let test_profile = "test-has-other-alone";
        let _ = delete_profile(test_profile);
        create_profile(test_profile).unwrap();

        register_instance(test_profile);
        assert!(!has_other_instances(test_profile));

        unregister_instance(test_profile);
        let _ = delete_profile(test_profile);
    }

    #[test]
    #[serial]
    fn test_cleanup_stale_instances() {
        let temp = tempdir().unwrap();
        setup_test_home(temp.path());

        let test_profile = "test-stale-cleanup";
        let _ = delete_profile(test_profile);
        create_profile(test_profile).unwrap();

        let base = get_app_dir().unwrap();
        let instances_dir = base.join("profiles").join(test_profile).join(".instances");
        fs::create_dir_all(&instances_dir).unwrap();

        // Write a fake PID file for a non-existent process
        let fake_pid = 99999999u32;
        fs::write(instances_dir.join(fake_pid.to_string()), "").unwrap();
        assert!(instances_dir.join(fake_pid.to_string()).exists());

        cleanup_stale_instances(test_profile);
        assert!(!instances_dir.join(fake_pid.to_string()).exists());

        let _ = delete_profile(test_profile);
    }

    #[test]
    #[serial]
    fn test_cleanup_empty_profile_respects_other_instances() {
        let temp = tempdir().unwrap();
        setup_test_home(temp.path());

        let test_profile = "test-cleanup-multi";
        let _ = delete_profile(test_profile);
        create_profile(test_profile).unwrap();

        // Register our instance
        register_instance(test_profile);

        let base = get_app_dir().unwrap();
        let instances_dir = base.join("profiles").join(test_profile).join(".instances");
        // Use parent PID as a fake "other instance" (always alive and accessible)
        let parent_pid = std::os::unix::process::parent_id();
        fs::write(instances_dir.join(parent_pid.to_string()), "").unwrap();

        // cleanup should not delete because has_other_instances returns true
        cleanup_empty_profile(test_profile);
        assert!(base.join("profiles").join(test_profile).exists());

        let _ = fs::remove_file(instances_dir.join(parent_pid.to_string()));
        unregister_instance(test_profile);

        // Now cleanup should delete (no sessions, no other instances)
        cleanup_empty_profile(test_profile);
        assert!(!base.join("profiles").join(test_profile).exists());
    }
}
