use super::*;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;

struct SearchPath(Option<OsString>);

impl Drop for SearchPath {
    fn drop(&mut self) {
        match &self.0 {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }
}

fn fixture(dir: &Path) -> SearchPath {
    let old = SearchPath(std::env::var_os("PATH"));
    let probe = "#!/bin/sh\n[ \"$1\" = --capabilities-json ] || exit 1\necho '{\"defer_registration\":1}'\n";
    for name in ["agy", "xats-agy", "xats-agy-mcp"] {
        let path = dir.join(name);
        std::fs::write(&path, probe).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("PATH", dir);
    old
}

#[test]
#[serial_test::serial]
fn antigravity_launch_preserves_slot_and_argv_in_each_team_mode() {
    let root = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(root.path());
    let dir = root.path().canonicalize().unwrap();
    let _path = fixture(&dir);
    let cwd = dir.to_str().unwrap();
    let mut inst = Instance::new("test", cwd);
    inst.source_profile = "custom-profile".into();
    let id = Uuid::new_v4().to_string();
    let key = Uuid::new_v4().to_string();
    let digest = format!("{:x}", Sha256::digest(key.as_bytes()));
    for team in [false, true] {
        let pane = PaneConfig::new("antigravity", cwd, true, team);
        inst.set_primary_pane_config(pane.clone());
        inst.command = shell_escape(dir.join("agy").to_str().unwrap());
        for primary in [false, true] {
            let build = |resume| {
                if team {
                    inst.prepare_controlled_antigravity(
                        &pane,
                        primary,
                        resume,
                        &key,
                        IdentityKeyOrigin::Minted,
                        "custom-profile",
                    )
                    .map(Option::unwrap)
                } else {
                    inst.antigravity_command(&pane, primary, resume, None, "custom-profile")
                }
            };
            let cmd = build(Some(&id)).unwrap();
            assert!(cmd.contains(&format!("--conversation {id}")));
            assert!(cmd.contains("--dangerously-skip-permissions"));
            assert!(cmd.contains("AGENT_OF_EMPIRES_PROFILE='custom-profile'"));
            assert!(cmd.contains(&format!("AOE_INSTANCE_ID='{}'", inst.id)));
            assert_eq!(cmd.starts_with("exec env "), team);
            assert_eq!(cmd.contains("__antigravity-launch -- "), team);
            assert_eq!(cmd.contains(" --defer-registration -- "), team);
            assert_eq!(cmd.contains("AOE_ANTIGRAVITY_NAMED=1"), team);
            assert_eq!(cmd.contains("AOE_ANTIGRAVITY_DEFERRED=1"), team);
            assert_eq!(
                cmd.contains(&format!("AOE_ANTIGRAVITY_SLOT_DIGEST={digest}")),
                team
            );
            assert!(!cmd.contains(&key));
            if team {
                let outer = shell_words::split(&cmd).unwrap();
                let args = shell_words::split(outer.last().unwrap()).unwrap();
                for value in [
                    format!("AOE_ANTIGRAVITY_MCP_BINARY={cwd}/xats-agy-mcp"),
                    format!("AOE_ANTIGRAVITY_MCP_PATH={cwd}"),
                ] {
                    assert!(args.contains(&value));
                }
            }
            assert!(build(Some("latest")).is_err());
            assert!(!build(None).unwrap().contains("--conversation"));
        }
        if team {
            let extra = inst
                .prepare_extra_pane_config_command("extra-profile", "unused", &pane)
                .unwrap();
            let digest = format!("{:x}", Sha256::digest(extra.identity_key.as_bytes()));
            assert!(extra.command.contains(&digest));
            assert!(extra
                .command
                .contains("AGENT_OF_EMPIRES_PROFILE='extra-profile'"));
            assert!(!extra.command.contains(&extra.identity_key));
        }
    }
}

#[test]
#[serial_test::serial]
fn later_slot_preflight_failure_preserves_keys_and_reports_the_entry() {
    let root = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(root.path());
    let _path = fixture(root.path());
    let mut inst = Instance::new("test", root.path().to_str().unwrap());
    inst.source_profile = "preflight".into();
    let store = crate::db::Store::open_with_schema(&inst.source_profile).unwrap();
    for i in 0..2 {
        let cwd = root.path().join(i.to_string());
        std::fs::create_dir_all(cwd.join(".agents")).unwrap();
        let config = PaneConfig::new("antigravity", cwd.to_str().unwrap(), false, true);
        store
            .upsert_agent_slot_config(
                &inst.id,
                i,
                &config,
                &Uuid::new_v4().to_string(),
                &format!("%{i}"),
                "",
                1,
            )
            .unwrap();
        if i == 1 {
            std::fs::write(
                cwd.join(".agents/mcp_config.json"),
                r#"{"mcpServers":{"duplicate":{"command":"xats-agy-mcp"}}}"#,
            )
            .unwrap();
        }
    }
    let mut slots = store.read_slots_for_instance(&inst.id).unwrap();
    let before = slots.clone();
    let origins = inst.ensure_slot_identity_keys(&store, &mut slots);
    let outcomes = inst.resume_all_tracked_panes(&slots, RestartMode::Resume, &origins);
    assert!(
        matches!(outcomes.as_slice(), [PaneResumeOutcome::Error(reason)]
        if reason.contains("slot 1") && reason.contains("duplicate"))
    );
    assert_eq!(store.read_slots_for_instance(&inst.id).unwrap(), before);
    assert!(!root.path().join(".gemini/config/hooks.json").exists());
}
