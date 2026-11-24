use anyhow::{ensure, Context, Result};
use std::path::Path;

pub(crate) mod agentapi;
pub(crate) mod launcher;
mod pane;
pub(crate) use pane::launcher_key;
pub(crate) use pane::{bridge_key, is_launch_descendant};

fn is_absolute_binary(binary: &str) -> bool {
    let path = Path::new(binary);
    path.is_absolute()
        && matches!(
            path.file_name().and_then(|s| s.to_str()),
            Some("agy" | "antigravity")
        )
}

pub(crate) fn is_native_binary(command: &str) -> bool {
    let Ok(words) = shell_words::split(command) else {
        return false;
    };
    let [binary] = words.as_slice() else {
        return false;
    };
    let command = command.trim();
    is_absolute_binary(binary)
        && (command == crate::session::environment::shell_escape(binary)
            || (command == binary
                && binary
                    .chars()
                    .all(|c| c.is_alphanumeric() || "/._+-".contains(c)))
            || (command.strip_prefix('"').and_then(|s| s.strip_suffix('"')) == Some(binary)
                && !binary.contains(['$', '`', '\\', '"'])))
}

pub(crate) fn validate_args(args: &[String]) -> Result<()> {
    for arg in args.iter().take_while(|arg| arg.as_str() != "--") {
        let flag = arg.split('=').next().unwrap_or(arg);
        ensure!(
            !matches!(
                flag,
                "--conversation" | "-conversation" | "--continue" | "-continue" | "-c" | "--c"
            ),
            "Antigravity argument '{flag}' selects a conversation; AoE selects it on restart"
        );
    }
    Ok(())
}

pub(crate) fn launch_arguments(
    command: Option<&str>,
    extra: Option<&str>,
    defaults: Option<&str>,
) -> Result<(String, String)> {
    let split = |text: &str| shell_words::split(text).context("Invalid Antigravity arguments");
    let mut args = split(defaults.filter(|_| extra.is_none()).unwrap_or_default())?;
    let mut executable = String::new();
    if let Some(command) = command {
        let words = split(command)?;
        let (binary, inline) = words
            .split_first()
            .context("Missing Antigravity executable")?;
        if binary.contains('/') {
            ensure!(
                is_absolute_binary(binary) && !binary.contains(['\r', '\n']),
                "Expected an absolute path to agy or antigravity: {binary}"
            );
            ensure!(
                super::executable::is_executable(Path::new(binary)),
                "Antigravity executable is missing or not executable: {binary}"
            );
            executable = crate::session::environment::shell_escape(binary);
        }
        args.extend_from_slice(inline);
    }
    args.extend(split(extra.unwrap_or_default())?);
    validate_args(&args)?;
    Ok((executable, shell_words::join(args)))
}

#[cfg(test)]
mod tests {
    use crate::agents::*;

    #[test]
    fn antigravity_only_treats_a_single_absolute_executable_as_native() {
        use crate::session::Instance;

        let mut instance = Instance::new("test", "/tmp");
        instance.tool = "antigravity".into();
        for native in ["/bin/agy", "'/some path/antigravity'", "\"/some path/agy\""] {
            instance.command = native.into();
            assert!(!instance.has_command_override(), "{native}");
        }
        for custom in [
            "./agy",
            "env /bin/agy",
            "/bin/helper-agy",
            "/bin/agy --model M",
            "/bin/$VERSION/agy",
            "/other/codex",
        ] {
            instance.command = custom.into();
            assert!(instance.has_command_override(), "{custom}");
        }
        instance.tool = "codex".into();
        for custom in ["/bin/agy", "/bin/codex"] {
            instance.command = custom.into();
            assert!(instance.has_command_override(), "{custom}");
        }
    }

    #[test]
    fn antigravity_matches_its_command_and_not_argument_substrings() {
        for cmd in ["antigravity", "AGY", "'/some path/agy' --agent codex"] {
            assert_eq!(resolve_tool_name(cmd), Some("antigravity"), "{cmd}");
        }
        for cmd in [
            "bagy",
            "agy-helper",
            "/tmp/agy/editor",
            "echo agy",
            "echo antigravity",
        ] {
            assert_eq!(resolve_tool_name(cmd), None, "{cmd}");
        }
        assert_eq!(resolve_tool_name("pi --model agy"), Some("pi"));
    }

    #[test]
    fn antigravity_native_launch_contract() {
        let agent = get_agent("antigravity").unwrap();
        assert_eq!(agent.binary, "agy");
        assert!(matches!(agent.detection, DetectionMethod::Which("agy")));
        assert!(matches!(
            agent.yolo,
            Some(YoloMode::CliFlag("--dangerously-skip-permissions"))
        ));
        assert!(agent.supports_host_launch && agent.supports_cross_agent_team);
        assert!(agent.exact_session_runtime.is_none() && !is_switchable_agent(agent.name));
        assert!(agent.fork_template.is_none() && agent.session_id_flag.is_none());
        let resume = agent.resume.unwrap();
        let pattern = regex::Regex::new(resume.resume_pattern).unwrap();
        let uuid = "01234567-89ab-cdef-0123-456789abcdef";
        for command in [
            format!("agy --conversation={uuid}"),
            format!("agy --conversation {uuid}"),
        ] {
            let output = format!("Resume with -c (or command below):\n{command}\r\n");
            let found = pattern.captures(&output).unwrap();
            assert_eq!(&found[1], uuid);
            assert_eq!(
                resume.resume_flag.replace("{}", &found[1]),
                format!("--conversation {uuid}")
            );
        }
        for command in [
            format!("agy --conversation={uuid}-bad"),
            format!("agy --continue {uuid}"),
            "agy --conversation=deadbeef".into(),
        ] {
            assert!(!pattern.is_match(&command), "{command}");
        }
    }
}
