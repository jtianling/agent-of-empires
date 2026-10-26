use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    version: u32,
    pid: u32,
    profile: String,
    started_at: String,
}

pub(crate) struct TuiRegistration {
    profile: String,
}

impl TuiRegistration {
    pub(crate) fn new(profile: &str) -> Result<Self> {
        register(profile).with_context(|| format!("Registering TUI profile '{profile}'"))?;
        Ok(Self {
            profile: profile.into(),
        })
    }

    pub(crate) fn switch_to(&mut self, profile: &str) -> Result<()> {
        if profile == self.profile {
            return Ok(());
        }
        validate_profile(profile)?;
        unregister(&self.profile).context("Removing previous TUI registration")?;
        if let Err(error) = register(profile) {
            register(&self.profile).with_context(|| format!(
                "Registering TUI profile '{profile}' failed: {error:#}; restoring old registration failed"
            ))?;
            return Err(error).with_context(|| format!("Registering TUI profile '{profile}'"));
        }
        self.profile = profile.into();
        Ok(())
    }
}

impl Drop for TuiRegistration {
    fn drop(&mut self) {
        if let Err(error) = unregister(&self.profile) {
            tracing::warn!(
                profile = self.profile,
                "Cannot remove TUI registration: {error:#}"
            );
        }
    }
}

fn unregister(profile: &str) -> Result<()> {
    let path = super::get_app_dir_path()?
        .join("profiles")
        .join(profile)
        .join(".instances")
        .join(std::process::id().to_string());
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Removing {}", path.display())),
    }
}

pub(crate) fn validate_profile(profile: &str) -> Result<()> {
    ensure!(
        !profile.is_empty()
            && profile != "."
            && profile != ".."
            && !profile.contains(['/', '\\'])
            && !profile.chars().any(char::is_control),
        "Invalid profile name"
    );
    Ok(())
}

pub(crate) fn profiles() -> Result<Vec<String>> {
    let path = super::get_app_dir_path()?.join("profiles");
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut profiles = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Invalid profile name encoding"))?;
            validate_profile(&name)?;
            profiles.push(name);
        }
    }
    profiles.sort();
    Ok(profiles)
}

pub(super) fn register(profile: &str) -> Result<()> {
    validate_profile(profile)?;
    let pid = std::process::id();
    let started_at =
        crate::process::start_identity(pid)?.context("Current process has no start identity")?;
    let registration = Registration {
        version: 1,
        pid,
        profile: profile.into(),
        started_at,
    };
    let directory = super::ensure_profile_dir(profile)?.join(".instances");
    fs::create_dir_all(&directory)?;
    super::storage_merge::atomic_write(
        &directory.join(pid.to_string()),
        &serde_json::to_vec(&registration)?,
    )
}

pub fn query() -> Result<Value> {
    let observed_at = chrono::Utc::now().to_rfc3339();
    let mut instances = Vec::new();
    let mut errors = Vec::new();
    for profile in profiles()? {
        match query_profile(&profile, &observed_at) {
            Ok(entries) => instances.extend(entries),
            Err(error) => {
                tracing::warn!(profile, "Cannot inspect TUI registrations: {error:#}");
                errors.push(json!({"profile":profile,"source":"tui_registration",
                    "message":format!("{error:#}")}));
            }
        }
    }
    let partial = !errors.is_empty()
        || instances
            .iter()
            .any(|entry| matches!(entry["status"].as_str(), Some("invalid" | "unavailable")));
    Ok(json!({"observed_at":observed_at,"partial":partial,
        "instances":instances,"errors":errors}))
}

fn query_profile(profile: &str, observed_at: &str) -> Result<Vec<Value>> {
    let directory = super::get_app_dir_path()?
        .join("profiles")
        .join(profile)
        .join(".instances");
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let pid = name
            .parse::<u32>()
            .ok()
            .filter(|pid| *pid > 0 && *pid <= i32::MAX as u32);
        let mut value = json!({"profile":profile,"pid":pid,"registration":name,
            "source":"tui_registration+process_start","observed_at":observed_at});
        match inspect(&entry.path(), profile, pid) {
            Ok((status, start, reason)) => {
                value["status"] = json!(status);
                value["started_at"] = json!(start);
                value["reason"] = json!(reason);
            }
            Err(error) => {
                tracing::warn!(profile, "Cannot inspect TUI registration: {error:#}");
                value["status"] = json!("invalid");
                value["started_at"] = Value::Null;
                value["reason"] = json!(format!("{error:#}"));
            }
        }
        result.push(value);
    }
    result.sort_by_key(|entry| entry["registration"].as_str().unwrap_or("").to_string());
    Ok(result)
}

fn inspect(
    path: &Path,
    profile: &str,
    pid: Option<u32>,
) -> Result<(&'static str, Option<String>, String)> {
    let pid = pid.context("Registration filename is not a valid PID")?;
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "Registration is not a regular file"
    );
    ensure!(
        fs::metadata(path)?.len() <= 65536,
        "TUI registration exceeds size limit"
    );
    let bytes = fs::read(path)?;
    let current = match crate::process::start_identity(pid) {
        Ok(current) => current,
        Err(error) => return Ok(("unavailable", None, format!("{error:#}"))),
    };
    if bytes.is_empty() {
        return Ok((
            if current.is_some() {
                "unverified"
            } else {
                "stale"
            },
            None,
            "Legacy registration has no process start identity".into(),
        ));
    }
    let registration: Registration =
        serde_json::from_slice(&bytes).context("Invalid TUI registration schema")?;
    ensure!(
        registration.version == 1
            && registration.pid == pid
            && registration.profile == profile
            && !registration.started_at.is_empty(),
        "TUI registration identity mismatch"
    );
    let matches = current.as_ref() == Some(&registration.started_at);
    Ok((
        if matches { "verified" } else { "stale" },
        Some(registration.started_at),
        if matches {
            "Process start identity matches"
        } else {
            "Process exited or PID was reused"
        }
        .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn registration_follows_profile_switch_and_drop() {
        let home = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(home.path());
        let mut registration = TuiRegistration::new("first").unwrap();
        let other = super::super::ensure_profile_dir("first")
            .unwrap()
            .join(".instances")
            .join("99999999");
        fs::write(&other, "").unwrap();
        registration.switch_to("second").unwrap();
        let result = query().unwrap();
        let current: Vec<_> = result["instances"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["pid"] == std::process::id())
            .collect();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0]["profile"], "second");
        assert_eq!(current[0]["status"], "verified");
        drop(registration);
        let result = query().unwrap();
        assert!(!result["instances"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["pid"] == std::process::id()));
        assert!(other.exists());
    }

    #[test]
    #[serial]
    fn failed_registration_preserves_previous_profile() {
        let home = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(home.path());
        let mut registration = TuiRegistration::new("first").unwrap();
        let blocked = super::super::ensure_profile_dir("blocked")
            .unwrap()
            .join(".instances");
        fs::write(blocked, "not a directory").unwrap();
        let error = registration.switch_to("blocked").unwrap_err();
        assert!(format!("{error:#}").contains("Registering TUI profile 'blocked'"));
        let result = query().unwrap();
        let first = result["instances"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["pid"] == std::process::id())
            .unwrap();
        assert_eq!(first["profile"], "first");
        assert_eq!(first["status"], "verified");
    }
}
