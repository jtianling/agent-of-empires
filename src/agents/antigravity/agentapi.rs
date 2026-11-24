use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use crate::agents::executable;
use crate::session::environment::shell_escape;

pub(crate) const AGENTAPI_FLAG: &str = "--agentapi";
pub(crate) const CHANNEL_BIND_BIN: &str = "xats-agy-channel-bind";
pub(crate) const CHANNEL_BIND_ALLOW_RULE: &str = "command(regex:^xats-agy-channel-bind$)";
pub(crate) const CHANNEL_BIND_PROMPT: &str =
    "Run xats-agy-channel-bind once via run_command to bind xats wakeup channel, then reply OK.";

const SETTINGS_REL_PATH: &str = ".gemini/antigravity-cli/settings.json";
const UPDATE_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) fn channel_bind_available() -> bool {
    executable::resolve(CHANNEL_BIND_BIN).is_some()
}

pub(crate) fn probe_agentapi_channel(launcher: &Path) -> bool {
    if !channel_bind_available() {
        return false;
    }
    crate::antigravity_mcp::probe_capabilities(launcher)
        .map(|caps| caps.agentapi_channel)
        .unwrap_or(false)
}

pub(crate) fn build_settings_json(home: &Path) -> Result<String> {
    let path = home.join(SETTINGS_REL_PATH);
    let mut permissions = load_existing_permissions(&path)?;
    let allow = permissions
        .entry("allow")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .context("Expected permissions.allow in Antigravity settings.json to be an array")?;
    if !allow
        .iter()
        .any(|entry| entry.as_str() == Some(CHANNEL_BIND_ALLOW_RULE))
    {
        allow.push(Value::String(CHANNEL_BIND_ALLOW_RULE.to_owned()));
    }
    serde_json::to_string(&json!({ "permissions": permissions }))
        .context("Serializing Antigravity --settings JSON")
}

fn load_existing_permissions(path: &Path) -> Result<Map<String, Value>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Reading Antigravity settings {}", path.display()))
        }
    };
    let doc: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Parsing Antigravity settings {}", path.display()))?;
    match doc.get("permissions") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => anyhow::bail!(
            "Expected permissions in {} to be a JSON object",
            path.display()
        ),
    }
}

pub(crate) fn agy_channel_args(include_handshake_prompt: bool) -> Result<String> {
    let home = dirs::home_dir().context("Cannot locate home directory for Antigravity settings")?;
    agy_channel_args_for_home(&home, include_handshake_prompt)
}

pub(crate) fn agy_channel_args_for_home(
    home: &Path,
    include_handshake_prompt: bool,
) -> Result<String> {
    let settings = build_settings_json(home)?;
    let mut args = format!(" --settings {}", shell_escape(&settings));
    if include_handshake_prompt {
        args.push_str(&format!(" -i {}", shell_escape(CHANNEL_BIND_PROMPT)));
    }
    Ok(args)
}

pub(crate) fn notify_conversation_update(conversation_id: &str) {
    if std::env::var("XATS_AGY_AGENTAPI").as_deref() != Ok("1") {
        return;
    }
    let Some(bin) = executable::resolve(CHANNEL_BIND_BIN) else {
        return;
    };
    let mut child = match Command::new(bin)
        .arg("--update-conversation")
        .env("ANTIGRAVITY_CONVERSATION_ID", conversation_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            tracing::debug!("Failed to spawn {CHANNEL_BIND_BIN} --update-conversation: {error}");
            return;
        }
    };
    let deadline = Instant::now() + UPDATE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::debug!("Timed out waiting for {CHANNEL_BIND_BIN} --update-conversation");
                return;
            }
            Err(error) => {
                tracing::debug!("Error polling {CHANNEL_BIND_BIN} --update-conversation: {error}");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_json_merges_existing_permissions_without_dropping_rules() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join(SETTINGS_REL_PATH);
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(
            &settings_path,
            json!({
                "statusLine": {"type": "command", "command": "~/.gemini/statusline.sh"},
                "permissions": {
                    "allow": ["command(git status)"],
                    "ask": ["command(rm)"],
                    "deny": ["command(sudo)"]
                }
            })
            .to_string(),
        )
        .unwrap();

        let encoded = build_settings_json(dir.path()).unwrap();
        let parsed: Value = serde_json::from_str(&encoded).unwrap();
        assert!(parsed.get("statusLine").is_none());
        assert_eq!(
            parsed["permissions"]["allow"],
            json!(["command(git status)", CHANNEL_BIND_ALLOW_RULE])
        );
        assert_eq!(parsed["permissions"]["ask"], json!(["command(rm)"]));
        assert_eq!(parsed["permissions"]["deny"], json!(["command(sudo)"]));

        // Deduplicates if already present.
        std::fs::write(&settings_path, &encoded).unwrap();
        let second: Value =
            serde_json::from_str(&build_settings_json(dir.path()).unwrap()).unwrap();
        assert_eq!(
            second["permissions"]["allow"],
            json!(["command(git status)", CHANNEL_BIND_ALLOW_RULE])
        );
    }

    #[test]
    fn settings_json_handles_missing_file_and_formats_args() {
        let dir = tempfile::tempdir().unwrap();
        let deferred = agy_channel_args_for_home(dir.path(), false).unwrap();
        assert!(deferred.contains("--settings "));
        assert!(!deferred.contains(" -i "));
        assert!(deferred.contains(CHANNEL_BIND_ALLOW_RULE));

        let named = agy_channel_args_for_home(dir.path(), true).unwrap();
        assert!(named.contains("--settings "));
        assert!(named.contains(" -i "));
        assert!(named.contains("xats-agy-channel-bind"));
    }
}
