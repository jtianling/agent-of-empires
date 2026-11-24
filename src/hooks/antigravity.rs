use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

const GROUP: &str = "agent-of-empires-pane-capture";
const CONFIG: &str = ".gemini/config/hooks.json";

pub(crate) fn install_for(pane: &crate::session::PaneConfig, sandbox: bool) -> Result<()> {
    if pane.tool == "antigravity" && !sandbox {
        let home = dirs::home_dir().context("Cannot locate Antigravity hook directory")?;
        change(&home.join(CONFIG), Some(&super::aoe_binary_path()))?;
        if pane.cross_agent_team {
            super::antigravity_xats::install(&home)?;
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
}
