//! Validation of the user's kimi MCP configuration.
//!
//! xats binds a kimi agent's identity from a header the MCP connection carries,
//! and kimi only fills that header from a template the user wrote. AoE checks
//! that the template is there and never writes the file: a wrong entry binds
//! two agents to one identity, and that failure is silent, so it is worth one
//! explicit confirmation from the person who owns the config.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const SESSION_ID_HEADER: &str = "X-Kimi-Session-Id";
const SESSION_ID_TEMPLATE: &str = "${KIMI_XATS_SESSION_ID}";
const XATS_SERVER_NAME: &str = "cross-agent-teams";

#[derive(Debug, Default, Deserialize)]
struct McpConfigFile {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: std::collections::BTreeMap<String, McpServerEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct McpServerEntry {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
}

/// Check the user-level `mcp.json` under the kimi home directory.
///
/// One user-level entry covers every pane: the template resolves through kimi's
/// per-session overlay, so each session-scoped connection expands it to its own
/// id. AoE therefore never generates per-pane configuration.
pub fn validate(home: &Path) -> Result<()> {
    let path = home.join("mcp.json");
    let config = read_config(&path)?;
    let entry = locate_entry(&config).ok_or_else(|| missing_entry_error(&path))?;
    let (name, entry) = entry;
    if entry.enabled == Some(false) {
        bail!("{}", disabled_error(&path, name));
    }
    let header = entry
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(SESSION_ID_HEADER));
    match header {
        Some((_, value)) if value.trim() == SESSION_ID_TEMPLATE => {}
        _ => bail!("{}", missing_header_error(&path, name)),
    }
    if entry.scope.as_deref() != Some("session") {
        bail!("{}", wrong_scope_error(&path, name, entry.scope.as_deref()));
    }
    Ok(())
}

/// The kimi MCP config path AoE reads, for callers that only need to name it.
pub fn config_path(home: &Path) -> PathBuf {
    home.join("mcp.json")
}

fn read_config(path: &Path) -> Result<McpConfigFile> {
    let metadata = std::fs::metadata(path).map_err(|_| missing_file_error(path))?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
        bail!(
            "kimi MCP configuration '{}' is not a readable file",
            path.display()
        );
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading kimi MCP configuration '{}'", path.display()))?;
    if text.trim().is_empty() {
        return Ok(McpConfigFile::default());
    }
    serde_json::from_str(&text).with_context(|| {
        format!(
            "kimi MCP configuration '{}' is not valid JSON",
            path.display()
        )
    })
}

/// The xats entry under its documented name, or the single entry that carries
/// the xats session header under a name the user chose.
fn locate_entry(config: &McpConfigFile) -> Option<(&str, &McpServerEntry)> {
    if let Some((name, entry)) = config.mcp_servers.get_key_value(XATS_SERVER_NAME) {
        return Some((name.as_str(), entry));
    }
    config
        .mcp_servers
        .iter()
        .find(|(_, entry)| {
            entry
                .headers
                .keys()
                .any(|key| key.eq_ignore_ascii_case(SESSION_ID_HEADER))
        })
        .map(|(name, entry)| (name.as_str(), entry))
}

fn missing_file_error(path: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "kimi MCP configuration '{}' does not exist.\n{}",
        path.display(),
        required_snippet()
    )
}

fn missing_entry_error(path: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "kimi MCP configuration '{}' declares no xats server.\n{}",
        path.display(),
        required_snippet()
    )
}

fn disabled_error(path: &Path, name: &str) -> String {
    format!(
        "kimi MCP server '{name}' in '{}' is disabled, so the pane would never \
         bind an xats identity. Remove \"enabled\": false.",
        path.display()
    )
}

fn missing_header_error(path: &Path, name: &str) -> String {
    format!(
        "kimi MCP server '{name}' in '{}' does not send \
         {SESSION_ID_HEADER}: {SESSION_ID_TEMPLATE}, so xats cannot tell which \
         session the connection belongs to.\n{}",
        path.display(),
        required_snippet()
    )
}

fn wrong_scope_error(path: &Path, name: &str, scope: Option<&str>) -> String {
    format!(
        "kimi MCP server '{name}' in '{}' declares scope {}, so every session in \
         the workspace would share one connection and one xats identity. It must \
         declare \"scope\": \"session\".\n{}",
        path.display(),
        scope.map_or_else(
            || "the default \"workspace\"".to_string(),
            |s| format!("\"{s}\"")
        ),
        required_snippet()
    )
}

/// Printed on every failure so the user can paste a working entry rather than
/// reconstruct one from the diagnostic.
fn required_snippet() -> String {
    format!(
        "Add this to the \"mcpServers\" object, keeping your daemon's port:\n\
         \x20 \"{XATS_SERVER_NAME}\": {{\n\
         \x20   \"url\": \"http://127.0.0.1:9100/mcp\",\n\
         \x20   \"scope\": \"session\",\n\
         \x20   \"headers\": {{ \"{SESSION_ID_HEADER}\": \"{SESSION_ID_TEMPLATE}\" }}\n\
         \x20 }}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(home: &Path, body: &str) {
        std::fs::write(home.join("mcp.json"), body).unwrap();
    }

    fn valid_body() -> &'static str {
        r#"{"mcpServers":{"cross-agent-teams":{"url":"http://127.0.0.1:9100/mcp",
            "scope":"session","headers":{"X-Kimi-Session-Id":"${KIMI_XATS_SESSION_ID}"}}}}"#
    }

    #[test]
    fn a_conforming_user_level_entry_passes() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), valid_body());
        validate(home.path()).unwrap();
    }

    #[test]
    fn a_missing_file_reports_the_snippet_to_paste() {
        let home = tempfile::tempdir().unwrap();
        let error = validate(home.path()).unwrap_err();
        let diagnostic = format!("{error:#}");
        assert!(diagnostic.contains("does not exist"));
        assert!(diagnostic.contains("\"scope\": \"session\""));
        assert!(diagnostic.contains(SESSION_ID_TEMPLATE));
        assert!(!home.path().join("mcp.json").exists());
    }

    #[test]
    fn a_config_without_an_xats_entry_is_refused_and_left_alone() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            r#"{"mcpServers":{"other":{"command":"other-server"}}}"#,
        );
        let before = std::fs::read_to_string(home.path().join("mcp.json")).unwrap();
        let error = validate(home.path()).unwrap_err();
        assert!(format!("{error:#}").contains("declares no xats server"));
        assert_eq!(
            std::fs::read_to_string(home.path().join("mcp.json")).unwrap(),
            before
        );
    }

    #[test]
    fn an_entry_without_the_session_header_is_refused() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            r#"{"mcpServers":{"cross-agent-teams":{"url":"http://127.0.0.1:9100/mcp",
                "scope":"session"}}}"#,
        );
        let error = validate(home.path()).unwrap_err();
        assert!(format!("{error:#}").contains("does not send X-Kimi-Session-Id"));
    }

    #[test]
    fn a_workspace_scoped_entry_is_refused_even_with_the_header() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            r#"{"mcpServers":{"cross-agent-teams":{"url":"http://127.0.0.1:9100/mcp",
                "headers":{"X-Kimi-Session-Id":"${KIMI_XATS_SESSION_ID}"}}}}"#,
        );
        let error = validate(home.path()).unwrap_err();
        let diagnostic = format!("{error:#}");
        assert!(diagnostic.contains("the default \"workspace\""));
        assert!(diagnostic.contains("share one connection"));
    }

    #[test]
    fn a_renamed_entry_carrying_the_header_is_still_the_xats_entry() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            r#"{"mcpServers":{"teams":{"url":"http://127.0.0.1:9100/mcp","scope":"session",
                "headers":{"x-kimi-session-id":"${KIMI_XATS_SESSION_ID}"}}}}"#,
        );
        validate(home.path()).unwrap();
    }

    #[test]
    fn a_disabled_entry_is_refused() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            r#"{"mcpServers":{"cross-agent-teams":{"url":"http://127.0.0.1:9100/mcp",
                "scope":"session","enabled":false,
                "headers":{"X-Kimi-Session-Id":"${KIMI_XATS_SESSION_ID}"}}}}"#,
        );
        let error = validate(home.path()).unwrap_err();
        assert!(format!("{error:#}").contains("is disabled"));
    }

    #[test]
    fn one_configuration_serves_every_pane() {
        // Nothing in validation depends on a pane, a session or a slot, which is
        // the property that lets a single user level file cover concurrent panes.
        let home = tempfile::tempdir().unwrap();
        write(home.path(), valid_body());
        assert_eq!(config_path(home.path()), home.path().join("mcp.json"));
        validate(home.path()).unwrap();
        validate(home.path()).unwrap();
    }
}
