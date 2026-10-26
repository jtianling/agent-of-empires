use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, ensure, Context, Result};
use clap::Args;
use serde_json::json;

use crate::session::{environment::shell_escape, Instance};

pub(crate) mod opencode;
pub(crate) mod target;

const TOKEN_ENV: &str = "XATS_MANAGED_MCP_TOKEN";

#[derive(Args)]
pub struct ManagedLaunchArgs {
    #[arg(long)]
    instance_id: String,
    #[arg(long, requires = "slot")]
    pane_id: Option<String>,
    #[arg(long, requires = "pane_id")]
    slot: Option<i64>,
    #[arg(long, default_value = "claude", value_parser = ["claude", "codex"])]
    tool: String,
    #[arg(long)]
    launch_id: String,
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

pub fn prepare(instance: &mut Instance) -> Result<()> {
    if instance.managed_xats.is_none() {
        return Ok(());
    }
    match instance.tool.as_str() {
        "kimi" => bail!(
            "Managed xats Kimi requires a per-session MCP token API; \
             the current shared-server integration does not provide one"
        ),
        "claude" => prepare_claude(instance),
        "opencode" | "codex" => Ok(()),
        _ => bail!("Unsupported managed xats runtime"),
    }
}

fn prepare_claude(instance: &mut Instance) -> Result<()> {
    let control = crate::xats_control::discover_control_plane()?;
    let endpoint = control.base_url.join("/mcp")?.to_string();
    let token = crate::managed_xats::mcp_token(instance)?;
    let mut headers = json!({"X-Xats-Managed-Token": token});
    if let Some(bearer) = &control.token {
        headers["Authorization"] = json!(format!("Bearer {}", bearer.expose()));
    }
    let channel = channel_name(instance)?;
    let config = json!({"mcpServers": {
        "cross-agent-teams-mcp": {
            "type": "http", "url": endpoint, "headers": headers
        },
        channel: {
            "type": "stdio", "command": "npx", "args": ["--yes", "--package",
                "cross-agent-teams-mcp@latest", "cross-agent-teams-channel",
                "--daemon-url", endpoint]
        }
    }});
    let path = config_path(instance)?;
    write_private(&path, &serde_json::to_vec(&config)?)?;
    Ok(())
}

pub fn command(instance: &Instance, original: &str) -> Result<String> {
    if instance.managed_xats.is_none() || instance.tool != "claude" {
        return Ok(original.to_string());
    }
    let path = config_path(instance)?;
    let path = path
        .to_str()
        .context("Managed MCP config path is not UTF-8")?;
    wrap_launch(
        instance,
        &format!("{original} --mcp-config {}", shell_escape(path)),
        "claude",
    )
}

pub(crate) fn wrap_pane_command(instance: &Instance, original: &str) -> Result<String> {
    if instance.managed_xats.is_none() {
        return Ok(original.to_string());
    }
    wrap_launch(instance, original, "codex")
}

fn wrap_launch(instance: &Instance, original: &str, tool: &str) -> Result<String> {
    let identity = instance
        .managed_xats
        .as_ref()
        .context("Missing managed identity")?;
    let claim = identity.claim.as_ref().context("Missing managed claim")?;
    let executable = std::env::current_exe()?;
    let executable = executable
        .to_str()
        .context("AoE executable path is not UTF-8")?;
    let profile = profile(instance);
    let (session_id, pane_args) = match &instance.managed_pane_target {
        Some(target) => (
            target.session_id.as_str(),
            format!(
                " --pane-id {} --slot {}",
                shell_escape(&target.pane_id),
                target.slot
            ),
        ),
        None => (
            instance.id.as_str(),
            format!(
                " --pane-id {} --slot 0",
                shell_escape(instance.primary_pane_id())
            ),
        ),
    };
    Ok(format!(
        "{} -p {} __managed-launch --instance-id {}{pane_args} --tool {} --launch-id {} -- \
         env {original}",
        shell_escape(executable),
        shell_escape(&profile),
        shell_escape(session_id),
        shell_escape(tool),
        shell_escape(&claim.fence.instance_id),
    ))
}

pub fn run_launch(profile: &str, args: ManagedLaunchArgs) -> Result<()> {
    let pane_id = args
        .pane_id
        .as_deref()
        .context("Managed launch pane ID is missing")?;
    let slot = args.slot.context("Managed launch slot is missing")?;
    let projected = target::load_pane(
        profile,
        &args.instance_id,
        pane_id,
        slot,
        Some(&args.launch_id),
        &args.tool,
    )?;
    let instance = &projected;
    ensure!(instance.tool == args.tool, "Managed launch tool mismatch");
    validate_launch_fence(instance, &args.launch_id)?;
    target::mark_ready(profile, instance)?;
    let (program, arguments) = args
        .command
        .split_first()
        .context("Missing launch command")?;
    let mut child = std::process::Command::new(program);
    child
        .args(arguments)
        .env_remove(crate::xats_control::IDENTITY_KEY_ENV)
        .env_remove(TOKEN_ENV);
    if args.tool == "claude" {
        child.env(TOKEN_ENV, crate::managed_xats::mcp_token(instance)?);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(child.exec()).context("Launching managed agent")
    }
    #[cfg(not(unix))]
    {
        let status = child.status().context("Launching managed Claude")?;
        ensure!(status.success(), "Managed Claude exited with {status}");
        Ok(())
    }
}

#[cfg(test)]
fn validate_launch_instance(instance: &Instance, launch_id: &str) -> Result<()> {
    ensure!(
        instance.tool == "claude",
        "Managed launch helper requires Claude"
    );
    validate_launch_fence(instance, launch_id)
}

fn validate_launch_fence(instance: &Instance, launch_id: &str) -> Result<()> {
    let claim = instance
        .managed_xats
        .as_ref()
        .and_then(|x| x.claim.as_ref())
        .context("Managed launch has no claim")?;
    ensure!(
        claim.fence.instance_id == launch_id,
        "Managed launch fence is stale"
    );
    Ok(())
}

pub fn cleanup(instance: &Instance) -> Result<()> {
    if instance.managed_xats.is_none() || instance.tool != "claude" {
        return Ok(());
    }
    match std::fs::remove_file(config_path(instance)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Removing managed Claude MCP config"),
    }
}

fn channel_name(instance: &Instance) -> Result<String> {
    let name = instance
        .cross_agent_team_channel
        .strip_prefix("server:")
        .unwrap_or("cross-agent-teams-channel");
    ensure!(
        !name.is_empty()
            && name != "cross-agent-teams-mcp"
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
        "Invalid managed Claude channel name"
    );
    Ok(name.to_string())
}

fn profile(instance: &Instance) -> String {
    if instance.source_profile.is_empty() {
        crate::session::resolve_profile(None)
    } else {
        instance.source_profile.clone()
    }
}

fn config_path(instance: &Instance) -> Result<PathBuf> {
    ensure!(
        instance.primary_pane_id().len() == 16
            && instance
                .primary_pane_id()
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "Invalid managed session ID"
    );
    Ok(crate::session::get_profile_dir(&profile(instance))?
        .join("managed-launch")
        .join(format!("{}.json", instance.primary_pane_id())))
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("Missing config parent")?)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .context("Creating managed MCP config")?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        if let Err(error) = std::fs::remove_file(&temporary) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Failed to remove managed config temporary file: {error}");
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed_claude() -> Instance {
        let mut instance = Instance::new("managed", "/tmp");
        instance.managed_xats = Some(
            serde_json::from_value(json!({
                "name": "worker", "team": "test",
                "launch_instance_id": "00000000-0000-4000-8000-000000000001",
                "claim": {
                    "fence": {
                        "claim_id": "00000000-0000-4000-8000-000000000002",
                        "instance_id": "00000000-0000-4000-8000-000000000001",
                        "generation": 1
                    },
                    "mcp_token": "private-token", "runtime": null
                }
            }))
            .unwrap(),
        );
        instance
    }

    #[test]
    fn stale_launch_cannot_read_current_generation_token() {
        let instance = managed_claude();
        assert!(
            validate_launch_instance(&instance, "00000000-0000-4000-8000-000000000003").is_err()
        );
        assert!(
            validate_launch_instance(&instance, "00000000-0000-4000-8000-000000000001").is_ok()
        );
    }

    #[test]
    fn unmanaged_and_other_runtimes_cannot_use_claude_launcher() {
        assert!(validate_launch_instance(&Instance::new("human", "/tmp"), "id").is_err());
        let mut instance = managed_claude();
        instance.tool = "codex".to_string();
        assert!(
            validate_launch_instance(&instance, "00000000-0000-4000-8000-000000000001").is_err()
        );
    }

    #[test]
    #[serial_test::serial]
    fn launch_command_preserves_assignments_and_quoted_arguments() {
        let directory = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(directory.path());
        let mut instance = managed_claude();
        instance.source_profile = "managed-test".to_string();
        let wrapped = command(
            &instance,
            "CUSTOM='a b' claude --append-system-prompt 'keep this text'",
        )
        .unwrap();
        let words = shell_words::split(&wrapped).unwrap();
        let separator = words.iter().position(|word| word == "--").unwrap();
        assert_eq!(
            &words[separator + 1..separator + 7],
            &[
                "env",
                "CUSTOM=a b",
                "claude",
                "--append-system-prompt",
                "keep this text",
                "--mcp-config"
            ]
        );
        assert!(!wrapped.contains("private-token"));
    }

    #[test]
    fn channel_cannot_replace_business_mcp_entry() {
        let mut instance = managed_claude();
        instance.cross_agent_team_channel = "server:cross-agent-teams-mcp".to_string();
        assert!(channel_name(&instance).is_err());
        instance.cross_agent_team_channel = "server:worker-channel".to_string();
        assert_eq!(channel_name(&instance).unwrap(), "worker-channel");
    }

    #[test]
    #[serial_test::serial]
    fn pane_wrapper_keeps_environment_outside_the_managed_helper() {
        let directory = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(directory.path());
        let mut instance = managed_claude();
        instance.source_profile = "managed-test".to_string();
        instance.command = "CUSTOM='a b' claude".to_string();
        instance.cross_agent_team = true;
        instance.sync_primary_pane_from_legacy();
        let full = instance
            .build_pane_command(instance.primary_pane_config(), None, true, None)
            .unwrap();
        let outer = shell_words::split(&full).unwrap();
        assert!(outer.contains(&format!("AOE_INSTANCE_ID={}", instance.id)));
        let shell_flag = outer.iter().position(|word| word == "-lc").unwrap();
        let script = &outer[shell_flag + 1];
        let inner = shell_words::split(script.split_once("exec env ").unwrap().1).unwrap();
        let separator = inner.iter().position(|word| word == "--").unwrap();
        assert_eq!(
            &inner[separator + 1..separator + 4],
            &["env", "CUSTOM=a b", "claude"]
        );
        assert!(!full.contains("private-token"));
    }

    #[test]
    #[cfg(unix)]
    fn private_config_replacement_does_not_follow_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let directory = tempfile::tempdir().unwrap();
        let victim = directory.path().join("other");
        let path = directory.path().join("config.json");
        std::fs::write(&victim, "keep").unwrap();
        symlink(&victim, &path).unwrap();
        write_private(&path, b"private").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "private");
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn pane_helper_keeps_parent_and_logical_identity_separate() {
        let mut instance = managed_claude();
        let parent = instance.id.clone();
        instance.id = "0123456789abcdef".into();
        instance.managed_pane_target = Some(crate::session::ManagedPaneTarget {
            session_id: parent.clone(),
            pane_id: instance.id.clone(),
            slot: 2,
        });
        let command = command(&instance, "claude").unwrap();
        let words = shell_words::split(&command).unwrap();
        let argument = |name| words.windows(2).find(|pair| pair[0] == name).unwrap()[1].clone();
        assert_eq!(argument("--instance-id"), parent);
        assert_eq!(argument("--pane-id"), instance.id);
        assert_eq!(argument("--slot"), "2");
        assert!(argument("--mcp-config").ends_with("0123456789abcdef.json"));
        assert!(!command.contains("private-token"));
    }

    #[test]
    fn promoted_primary_keeps_its_existing_private_config_name() {
        let mut instance = managed_claude();
        instance.managed_primary_id = Some("0123456789abcdef".into());
        let command = command(&instance, "claude").unwrap();
        let words = shell_words::split(&command).unwrap();
        assert!(words.windows(2).any(|pair| pair == ["--slot", "0"]));
        assert!(words
            .windows(2)
            .any(|pair| pair == ["--pane-id", "0123456789abcdef"]));
        assert!(words
            .iter()
            .any(|word| word.ends_with("0123456789abcdef.json")));
    }

    #[test]
    fn original_primary_launches_include_the_exact_pane_target() {
        let instance = managed_claude();
        for command in [
            command(&instance, "claude").unwrap(),
            wrap_pane_command(&instance, "codex --remote ws://127.0.0.1:8799").unwrap(),
        ] {
            let words = shell_words::split(&command).unwrap();
            let argument = |name| words.windows(2).find(|pair| pair[0] == name).unwrap()[1].clone();
            assert_eq!(argument("--instance-id"), instance.id);
            assert_eq!(argument("--pane-id"), instance.id);
            assert_eq!(argument("--slot"), "0");
            assert!(!command.contains("private-token"));
        }
    }

    #[test]
    fn legacy_helper_without_pane_identity_fails_before_loading_state() {
        let error = run_launch(
            "unused",
            ManagedLaunchArgs {
                instance_id: "0123456789abcdef".into(),
                pane_id: None,
                slot: None,
                tool: "claude".into(),
                launch_id: uuid::Uuid::new_v4().to_string(),
                command: Vec::new(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("pane ID is missing"));
    }
}
