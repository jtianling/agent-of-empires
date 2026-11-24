use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::json;

const CONFIG: &str = ".gemini/config/mcp_config.json";
const SERVER: &str = "aoe-xats";
const COMMAND: &str = "__antigravity-mcp";

#[derive(Deserialize, Default)]
struct McpConfig {
    #[serde(default, rename = "mcpServers")]
    servers: BTreeMap<String, McpServer>,
}

#[derive(Deserialize, Default)]
struct McpServer {
    #[serde(default)]
    disabled: bool,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
}

pub(crate) fn check_named_mcp(cwd: &str) -> Result<()> {
    let home = dirs::home_dir().context("Cannot locate Antigravity config")?;
    check_mcp_files(
        &home.join(CONFIG),
        &Path::new(cwd).join(".agents/mcp_config.json"),
    )
}

fn check_mcp_files(global: &Path, project: &Path) -> Result<()> {
    for path in [global, project] {
        let Some(bytes) = super::antigravity_config::read(path)? else {
            continue;
        };
        let config: McpConfig = serde_json::from_slice(&bytes)
            .with_context(|| format!("Reading Antigravity MCP entries in {}", path.display()))?;
        for (name, server) in config.servers {
            if path == project && name == SERVER {
                anyhow::bail!(
                    "Workspace MCP entry {SERVER} shadows the AoE bridge in {}",
                    path.display()
                );
            }
            if server.disabled {
                continue;
            }
            let duplicate = server.command.iter().chain(&server.args).any(|arg| {
                matches!(
                    Path::new(arg).file_name().and_then(|s| s.to_str()),
                    Some("xats-agy-mcp" | "agy-mcp.js")
                )
            });
            let aoe_alias = name != SERVER && server.args.iter().any(|arg| arg == COMMAND);
            ensure!(!duplicate && !aoe_alias, "Antigravity MCP entry {name} in {} also registers the PTY launcher; use only {SERVER} in AoE", path.display());
        }
    }
    Ok(())
}

pub(crate) fn install(home: &Path) -> Result<()> {
    update(home, true)
}

pub(super) fn uninstall(home: &Path) -> Result<()> {
    update(home, false)
}

fn update(home: &Path, install: bool) -> Result<()> {
    let path = home.join(CONFIG);
    let entry = json!({"command": std::env::current_exe()?, "args": [COMMAND]});
    super::antigravity_config::update(&path, install, |config| {
        if !config.contains_key("mcpServers") && !install {
            return Ok(false);
        }
        let servers = config
            .entry("mcpServers")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("Expected mcpServers to be an object")?;
        if let Some(existing) = servers.get(SERVER) {
            ensure!(
                existing == &entry,
                "Antigravity already has a different {SERVER} MCP entry"
            );
            if install {
                return Ok(false);
            }
        }
        if install {
            servers.insert(SERVER.to_owned(), entry);
            Ok(true)
        } else {
            Ok(servers.remove(SERVER).is_some())
        }
    })
    .with_context(|| format!("Updating Antigravity MCP configuration {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;

    #[test]
    fn named_launch_rejects_a_second_registrar_before_start() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let project = root.path().join("project.json");
        fs::write(
            &global,
            r#"{"mcpServers":{"mine":{"command":"xats-agy-mcp","disabled":true}}}"#,
        )
        .unwrap();
        assert!(check_mcp_files(&global, &project).is_ok());
        fs::write(
            &project,
            r#"{"mcpServers":{"alternate":{"command":"node","args":["/tmp/dist/agy-mcp.js"]}}}"#,
        )
        .unwrap();
        assert!(check_mcp_files(&global, &project).is_err());
        fs::write(&project, r#"{"mcpServers":{"aoe-xats":{"disabled":true}}}"#).unwrap();
        assert!(check_mcp_files(&global, &project).is_err());
    }

    #[test]
    fn antigravity_mcp_config_preserves_other_entries_and_custom_edits() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        let path = root.join(CONFIG);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = json!({"other": 42, "mcpServers":{"mine":{"command":"mine"}}});
        fs::write(&path, original.to_string()).unwrap();
        install(root).unwrap();
        let installed = fs::read(&path).unwrap();
        install(root).unwrap();
        assert_eq!(fs::read(&path).unwrap(), installed);
        uninstall(root).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            original
        );
        let mut custom: Value = serde_json::from_slice(&installed).unwrap();
        custom["mcpServers"][SERVER]["command"] = "/user/wrapper".into();
        fs::write(&path, custom.to_string()).unwrap();
        assert!(install(root).is_err());
        assert!(uninstall(root).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), custom.to_string());
    }
}
