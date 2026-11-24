use std::path::Path;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

const GROUP: &str = "agent-of-empires-pane-capture";
const CONFIG: &str = ".gemini/config/hooks.json";
const SETTINGS: &str = ".gemini/antigravity-cli/settings.json";
const TRUSTED_WORKSPACES: &str = "trustedWorkspaces";
const TRUST_UPDATE_RETRIES: usize = 3;

pub(crate) fn install_for(pane: &crate::session::PaneConfig, sandbox: bool) -> Result<()> {
    if pane.tool == "antigravity" && !sandbox {
        let home = dirs::home_dir().context("Cannot locate Antigravity hook directory")?;
        change(&home.join(CONFIG), Some(&super::aoe_binary_path()))?;
        trust_workspace(&home, &pane.working_dir)?;
        if pane.cross_agent_team {
            super::antigravity_xats::install(&home)?;
        }
    }
    Ok(())
}

fn normalized_workspace_path(working_dir: &str) -> Option<String> {
    let trimmed = working_dir.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = Path::new(trimmed);
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy();
    let cleaned = if text.len() > 1 {
        text.trim_end_matches('/')
    } else {
        text.as_ref()
    };
    Some(cleaned.to_owned())
}

pub(crate) fn trust_workspace(home: &Path, working_dir: &str) -> Result<()> {
    let Some(workspace) = normalized_workspace_path(working_dir) else {
        return Ok(());
    };
    let path = home.join(SETTINGS);
    for attempt in 0..TRUST_UPDATE_RETRIES {
        let result = super::antigravity_config::update(&path, true, |settings| {
            let entry = settings
                .entry(TRUSTED_WORKSPACES)
                .or_insert_with(|| json!([]));
            let list = entry
                .as_array_mut()
                .context("Expected trustedWorkspaces to be an array")?;
            if list.iter().any(|item| item.as_str() == Some(&workspace)) {
                return Ok(false);
            }
            list.push(Value::String(workspace.clone()));
            Ok(true)
        });
        match result {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < TRUST_UPDATE_RETRIES
                    && error.to_string().contains("changed during update") =>
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Updating Antigravity trustedWorkspaces in {}",
                        path.display()
                    )
                });
            }
        }
    }
    Ok(())
}

pub(super) fn uninstall(home: &Path) -> Result<()> {
    change(&home.join(CONFIG), None)
}

fn entry(binary: &str) -> Value {
    let command = format!(
        "{} __record-pane --agent antigravity",
        shell_words::quote(binary)
    );
    json!({ "SessionStart": [{"type": "command", "command": command}] })
}

fn owned(group: &Value) -> bool {
    let Some(command) = group
        .pointer("/SessionStart/0/command")
        .and_then(Value::as_str)
    else {
        return false;
    };
    let Ok(args) = shell_words::split(command) else {
        return false;
    };
    args.len() == 4
        && args[1..] == ["__record-pane", "--agent", "antigravity"]
        && entry(&args[0]) == *group
}

fn change(path: &Path, binary: Option<&str>) -> Result<()> {
    super::antigravity_config::update(path, binary.is_some(), |groups| {
        if let Some(old) = groups.get(GROUP) {
            ensure!(
                owned(old),
                "Custom Antigravity hook group already uses {GROUP}"
            );
        }
        let Some(bin) = binary else {
            return Ok(groups.remove(GROUP).is_some());
        };
        let new = entry(bin);
        if groups.get(GROUP) == Some(&new) {
            return Ok(false);
        }
        groups.insert(GROUP.into(), new);
        Ok(true)
    })
}

pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 36
        && uuid::Uuid::try_parse(id)
            .is_ok_and(|uuid| !uuid.is_nil() && uuid.to_string().eq_ignore_ascii_case(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn antigravity_hook_merges_flat_session_start_and_preserves_custom_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = json!({"another": {"SessionStart": [{"command": "true"}]}});
        fs::write(&path, original.to_string()).unwrap();
        let bin = "/tmp/a 'quoted' path/aoe";
        change(&path, Some(bin)).unwrap();
        let installed = fs::read(&path).unwrap();
        change(&path, Some(bin)).unwrap();
        assert_eq!(installed, fs::read(&path).unwrap());
        let mut doc: Value = serde_json::from_slice(&installed).unwrap();
        assert_eq!(doc[GROUP], entry(bin));
        change(&path, None).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            original
        );
        doc[GROUP]["enabled"] = false.into();
        fs::write(&path, doc.to_string()).unwrap();
        assert!(change(&path, Some(bin)).is_err());
        assert!(change(&path, None).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), doc.to_string());
    }

    #[test]
    fn antigravity_trust_workspace_merges_canonical_path_and_preserves_settings() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("project");
        fs::create_dir_all(&work).unwrap();
        let canonical = work.canonicalize().unwrap().to_string_lossy().into_owned();
        let settings = dir.path().join(SETTINGS);
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(
            &settings,
            json!({"editor": "vim", "trustedWorkspaces": ["/existing"]}).to_string(),
        )
        .unwrap();

        trust_workspace(dir.path(), work.to_str().unwrap()).unwrap();
        let after_first = fs::read(&settings).unwrap();
        trust_workspace(dir.path(), &format!("{}/", work.display())).unwrap();
        assert_eq!(fs::read(&settings).unwrap(), after_first);

        let doc: Value = serde_json::from_slice(&after_first).unwrap();
        assert_eq!(doc["editor"], "vim");
        assert_eq!(doc["trustedWorkspaces"], json!(["/existing", canonical]));

        fs::write(
            &settings,
            json!({"trustedWorkspaces": "invalid"}).to_string(),
        )
        .unwrap();
        assert!(trust_workspace(dir.path(), work.to_str().unwrap()).is_err());
    }
}
