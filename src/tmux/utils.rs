//! tmux utility functions

use std::process::Command;

use crate::session::{
    config::{load_config, SortOrder},
    flatten_tree, Group, GroupTree, Instance, Item,
};

/// Hook index used for the aoe-specific `client-session-changed` hook that
/// keeps `Ctrl+b d` mapped to "go back" inside managed sessions.
const NESTED_DETACH_HOOK: &str = "client-session-changed[99]";
const AOE_PROFILE_OPTION: &str = "@aoe_profile";
const AOE_ORIGIN_PROFILE_OPTION_PREFIX: &str = "@aoe_origin_profile_";
const AOE_RETURN_SESSION_OPTION_PREFIX: &str = "@aoe_return_session_";
const AOE_LAST_DETACHED_SESSION_OPTION_PREFIX: &str = "@aoe_last_detached_session_";

/// Sets up a tmux hook that dynamically rebinds `Ctrl+b d` based on the
/// current session:
///
/// - **aoe sessions** (`aoe_*`, `aoe_term_*`, `aoe_cterm_*`): delegates to
///   `aoe tmux refresh-bindings` which sets d/j/k via `Command::new("tmux")`
///   (bypasses tmux's internal parser to avoid quoting issues).
/// - **Other sessions**: normal `detach-client` behavior is restored.
pub fn setup_nested_detach_binding(
    profile: &str,
    return_session: Option<&str>,
    client_name: Option<&str>,
) {
    store_client_attach_context(profile, return_session, client_name);

    let aoe_bin = shell_escape(&aoe_bin_path());
    let hook_cmd = format!(
        r##"if-shell -F "#{{m:aoe_*,#{{session_name}}}}" "run-shell '{aoe_bin} tmux refresh-bindings --client-name #{{client_name}}'" "bind-key d detach-client ; unbind-key j ; unbind-key k""##,
    );
    Command::new("tmux")
        .args(["set-hook", "-g", NESTED_DETACH_HOOK, &hook_cmd])
        .output()
        .ok();

    // Apply bindings immediately for the current managed session.
    apply_managed_session_bindings(client_name);
}

/// Removes the hook installed by [`setup_nested_detach_binding`] and restores
/// the default `Ctrl+b d` binding.
pub fn cleanup_nested_detach_binding() {
    Command::new("tmux")
        .args(["set-hook", "-gu", NESTED_DETACH_HOOK])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "d", "detach-client"])
        .output()
        .ok();
    cleanup_session_cycle_bindings();
}

fn aoe_bin_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "aoe".to_string())
}

fn session_cycle_run_shell_cmds(profile: &str) -> (String, String) {
    let aoe_bin = aoe_bin_path();
    let escaped = shell_escape(&aoe_bin);
    let escaped_profile = shell_escape(profile);
    let next = format!(
        "client_name=\"#{{client_name}}\"; {escaped} tmux switch-session --direction next --profile {escaped_profile} --client-name \"$client_name\""
    );
    let prev = format!(
        "client_name=\"#{{client_name}}\"; {escaped} tmux switch-session --direction prev --profile {escaped_profile} --client-name \"$client_name\""
    );
    (next, prev)
}

fn detach_run_shell_cmd() -> String {
    format!(
        concat!(
            "client_name=\"#{{client_name}}\"; ",
            "client_key=$(printf '%s' \"$client_name\" | tr -c '[:alnum:]' '_'); ",
            "tmux set-option -gq \"{}${{client_key}}\" \"#{{session_name}}\"; ",
            "target=$(tmux show-option -gv \"{}${{client_key}}\" 2>/dev/null); ",
            "if [ -n \"$target\" ]; then ",
            "tmux switch-client -c \"$client_name\" -t \"$target\" 2>/dev/null || tmux detach-client -t \"$client_name\"; ",
            "else ",
            "tmux switch-client -c \"$client_name\" -l 2>/dev/null || tmux detach-client -t \"$client_name\"; ",
            "fi"
        ),
        AOE_LAST_DETACHED_SESSION_OPTION_PREFIX,
        AOE_RETURN_SESSION_OPTION_PREFIX
    )
}

fn cycle_run_shell_cmd(direction: &str) -> String {
    let aoe_bin = shell_escape(&aoe_bin_path());
    format!(
        concat!(
            "client_name=\"#{{client_name}}\"; ",
            "client_key=$(printf '%s' \"$client_name\" | tr -c '[:alnum:]' '_'); ",
            "profile=$(tmux show-option -gv \"{}${{client_key}}\" 2>/dev/null); ",
            "if [ -n \"$profile\" ]; then ",
            "{} tmux switch-session --direction {} --profile \"$profile\" --client-name \"$client_name\"; ",
            "fi"
        ),
        AOE_ORIGIN_PROFILE_OPTION_PREFIX,
        aoe_bin,
        direction
    )
}

fn apply_managed_session_bindings(client_name: Option<&str>) {
    let detach_cmd = detach_run_shell_cmd();
    let next_cmd = cycle_run_shell_cmd("next");
    let prev_cmd = cycle_run_shell_cmd("prev");
    Command::new("tmux")
        .args(["bind-key", "d", "run-shell", &detach_cmd])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "j", "run-shell", &next_cmd])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "k", "run-shell", &prev_cmd])
        .output()
        .ok();
    let _ = client_name;
}

pub fn refresh_bindings(client_name: Option<&str>) -> anyhow::Result<()> {
    let session_name = current_tmux_session_name(client_name)?;
    let is_managed = session_name
        .as_deref()
        .map(|name| name.starts_with("aoe_"))
        .unwrap_or(false);

    if is_managed {
        apply_managed_session_bindings(client_name);
    } else {
        Command::new("tmux")
            .args(["bind-key", "d", "detach-client"])
            .output()
            .ok();
        cleanup_session_cycle_bindings();
    }
    Ok(())
}

fn store_client_attach_context(
    profile: &str,
    return_session: Option<&str>,
    client_name: Option<&str>,
) {
    let Some(client_name) = client_name
        .map(str::to_owned)
        .or_else(crate::tmux::get_current_client_name)
    else {
        return;
    };

    let profile_key = client_context_option_key(AOE_ORIGIN_PROFILE_OPTION_PREFIX, &client_name);
    Command::new("tmux")
        .args(["set-option", "-gq"])
        .arg(&profile_key)
        .arg(profile)
        .output()
        .ok();

    if let Some(return_session) = return_session {
        let return_key = client_context_option_key(AOE_RETURN_SESSION_OPTION_PREFIX, &client_name);
        Command::new("tmux")
            .args(["set-option", "-gq"])
            .arg(&return_key)
            .arg(return_session)
            .output()
            .ok();
    }
}

pub fn clear_last_detached_session_for_client(client_name: &str) {
    let option_key =
        client_context_option_key(AOE_LAST_DETACHED_SESSION_OPTION_PREFIX, client_name);
    Command::new("tmux")
        .args(["set-option", "-gu"])
        .arg(&option_key)
        .output()
        .ok();
}

pub fn set_last_detached_session_for_client(client_name: &str, session_name: &str) {
    let option_key =
        client_context_option_key(AOE_LAST_DETACHED_SESSION_OPTION_PREFIX, client_name);
    Command::new("tmux")
        .args(["set-option", "-gq"])
        .arg(&option_key)
        .arg(session_name)
        .output()
        .ok();
}

/// Consume the last managed session visited by a client. This value is used
/// only to restore TUI selection after nested detach, and is intentionally
/// separate from the immutable attach-origin AoE return target stored in
/// `@aoe_return_session_<client>`.
pub fn take_last_detached_session_for_client(client_name: &str) -> Option<String> {
    let option_key =
        client_context_option_key(AOE_LAST_DETACHED_SESSION_OPTION_PREFIX, client_name);
    let output = Command::new("tmux")
        .args(["show-option", "-gv"])
        .arg(&option_key)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let session_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    clear_last_detached_session_for_client(client_name);

    if session_name.is_empty() {
        None
    } else {
        Some(session_name)
    }
}

fn client_context_option_key(prefix: &str, client_name: &str) -> String {
    format!("{prefix}{}", sanitize_tmux_option_suffix(client_name))
}

fn sanitize_tmux_option_suffix(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Binds `Ctrl+b j` / `Ctrl+b k` to cycle through aoe agent sessions
/// belonging to the given profile. Works in both nested and non-nested tmux modes.
pub fn setup_session_cycle_bindings(profile: &str) {
    tag_sessions_with_profile(profile);

    let (switch_next, switch_prev) = session_cycle_run_shell_cmds(profile);
    Command::new("tmux")
        .args(["bind-key", "j", "run-shell", &switch_next])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "k", "run-shell", &switch_prev])
        .output()
        .ok();
}

fn tag_sessions_with_profile(profile: &str) {
    let Ok(storage) = crate::session::Storage::new(profile) else {
        return;
    };
    let Ok(instances) = storage.load() else {
        return;
    };
    for instance in &instances {
        let name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
        Command::new("tmux")
            .args(["set-option", "-t", &name, AOE_PROFILE_OPTION, profile])
            .output()
            .ok();
    }
}

pub fn cleanup_session_cycle_bindings() {
    Command::new("tmux").args(["unbind-key", "j"]).output().ok();
    Command::new("tmux").args(["unbind-key", "k"]).output().ok();
}

fn shell_escape(s: &str) -> String {
    if s.contains(|c: char| c.is_whitespace() || c == '\'' || c == '"') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

pub fn switch_aoe_session(
    direction: &str,
    profile: &str,
    client_name: Option<&str>,
) -> anyhow::Result<()> {
    let storage = crate::session::Storage::new(profile)?;
    let (instances, groups) = storage.load_with_groups()?;
    let Some(current) = current_tmux_session_name(client_name)? else {
        return Ok(());
    };
    let sessions = ordered_profile_sessions_for_cycle(
        &instances,
        &groups,
        current_home_sort_order(),
        &current,
    );

    if sessions.len() <= 1 {
        return Ok(());
    }

    let Some(target_session) = resolve_cycle_target(&sessions, &current, direction) else {
        return Ok(());
    };

    if let Some(client_name) = client_name {
        // Remember the actual managed session the user cycled to so TUI re-entry
        // can restore selection there without rewriting the AoE return target.
        set_last_detached_session_for_client(client_name, &target_session);
    }

    switch_client_to_session(&target_session, client_name)?;

    Ok(())
}

fn current_home_sort_order() -> SortOrder {
    load_config()
        .ok()
        .flatten()
        .and_then(|config| config.app_state.sort_order)
        .unwrap_or_default()
}

fn ordered_profile_sessions_for_cycle(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
    current: &str,
) -> Vec<String> {
    ordered_scoped_profile_session_names(instances, groups, sort_order, current)
        .into_iter()
        .filter(|name| tmux_session_exists(name))
        .collect()
}

fn ordered_scoped_profile_session_names(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
    current: &str,
) -> Vec<String> {
    let ordered = ordered_profile_session_names(instances, groups, sort_order);
    if !ordered.iter().any(|session_name| session_name == current) {
        return Vec::new();
    }

    let Some(current_instance) = instance_for_tmux_session_name(instances, current) else {
        return Vec::new();
    };

    ordered
        .into_iter()
        .filter(|session_name| {
            instance_for_tmux_session_name(instances, session_name)
                .is_some_and(|instance| instance.group_path == current_instance.group_path)
        })
        .collect()
}

fn ordered_profile_session_names(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
) -> Vec<String> {
    let group_tree = GroupTree::new_with_groups(instances, groups);
    flatten_tree(&group_tree, instances, sort_order)
        .into_iter()
        .filter_map(|item| match item {
            Item::Session { id, .. } => instances
                .iter()
                .find(|instance| instance.id == id)
                .map(|instance| crate::tmux::Session::generate_name(&instance.id, &instance.title)),
            Item::Group { .. } => None,
        })
        .collect()
}

fn instance_for_tmux_session_name<'a>(
    instances: &'a [Instance],
    tmux_session_name: &str,
) -> Option<&'a Instance> {
    instances
        .iter()
        .find(|instance| matches_managed_tmux_name(instance, tmux_session_name))
}

fn matches_managed_tmux_name(instance: &Instance, tmux_session_name: &str) -> bool {
    crate::tmux::Session::generate_name(&instance.id, &instance.title) == tmux_session_name
        || crate::tmux::TerminalSession::generate_name(&instance.id, &instance.title)
            == tmux_session_name
        || crate::tmux::ContainerTerminalSession::generate_name(&instance.id, &instance.title)
            == tmux_session_name
}

fn current_tmux_session_name(client_name: Option<&str>) -> anyhow::Result<Option<String>> {
    if let Some(client_name) = client_name {
        if let Some(session_name) = session_name_for_client(client_name)? {
            return Ok(Some(session_name));
        }
    }

    Ok(crate::tmux::get_current_session_name())
}

fn session_name_for_client(client_name: &str) -> anyhow::Result<Option<String>> {
    let output = Command::new("tmux")
        .args(["list-clients", "-F", "#{client_name}\t#{session_name}"])
        .output()?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_session_name_for_client(&stdout, client_name))
}

fn parse_session_name_for_client(stdout: &str, client_name: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let (listed_client, session_name) = line.split_once('\t')?;
        (listed_client == client_name).then(|| session_name.to_string())
    })
}

fn switch_client_to_session(target_session: &str, client_name: Option<&str>) -> anyhow::Result<()> {
    let mut command = Command::new("tmux");
    command.arg("switch-client");
    if let Some(client_name) = client_name {
        command.args(["-c", client_name]);
    }
    command.args(["-t", target_session]);
    command.output()?;
    Ok(())
}

fn resolve_cycle_target(sessions: &[String], current: &str, direction: &str) -> Option<String> {
    let current_idx = sessions.iter().position(|session| session == current)?;

    let target_idx = match direction {
        "next" => (current_idx + 1) % sessions.len(),
        "prev" => {
            if current_idx == 0 {
                sessions.len() - 1
            } else {
                current_idx - 1
            }
        }
        _ => return None,
    };

    sessions.get(target_idx).cloned()
}

fn tmux_session_exists(name: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn strip_ansi(content: &str) -> String {
    let mut result = content.to_string();

    while let Some(start) = result.find("\x1b[") {
        let rest = &result[start + 2..];
        let end_offset = rest
            .find(|c: char| c.is_ascii_alphabetic())
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        result = format!("{}{}", &result[..start], &result[start + 2 + end_offset..]);
    }

    while let Some(start) = result.find("\x1b]") {
        if let Some(end) = result[start..].find('\x07') {
            result = format!("{}{}", &result[..start], &result[start + end + 1..]);
        } else {
            break;
        }
    }

    result
}

pub fn sanitize_session_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(20)
        .collect()
}

/// Append `; set-option -p -t <target> remain-on-exit on` to an in-flight
/// tmux argument list so that remain-on-exit is set atomically with session
/// creation. Using pane-level (`-p`) avoids bleeding into user-created panes
/// in the same session.
///
/// Note: the `-p` (pane-level) flag requires tmux >= 3.0.
pub fn append_remain_on_exit_args(args: &mut Vec<String>, target: &str) {
    args.extend([
        ";".to_string(),
        "set-option".to_string(),
        "-p".to_string(),
        "-t".to_string(),
        target.to_string(),
        "remain-on-exit".to_string(),
        "on".to_string(),
    ]);
}

pub fn get_agent_pane_id(session_name: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["show-option", "-t", session_name, "-v", "@aoe_agent_pane"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn append_store_pane_id_args(args: &mut Vec<String>, target: &str) {
    args.extend([
        ";".to_string(),
        "set-option".to_string(),
        "-t".to_string(),
        target.to_string(),
        "@aoe_agent_pane".to_string(),
        "#{pane_id}".to_string(),
    ]);
}

fn resolve_pane_target(session_name: &str) -> String {
    get_agent_pane_id(session_name).unwrap_or_else(|| session_name.to_string())
}

pub fn is_pane_dead(session_name: &str) -> bool {
    let target = resolve_pane_target(session_name);
    Command::new("tmux")
        .args(["display-message", "-t", &target, "-p", "#{pane_dead}"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

fn pane_current_command(session_name: &str) -> Option<String> {
    let target = resolve_pane_target(session_name);
    Command::new("tmux")
        .args([
            "display-message",
            "-t",
            &target,
            "-p",
            "#{pane_current_command}",
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// Shells that indicate the agent is not running (the pane was restored by
// tmux-resurrect, the agent crashed back to a prompt, or the user exited).
const KNOWN_SHELLS: &[&str] = &[
    "bash", "zsh", "sh", "fish", "dash", "ksh", "tcsh", "csh", "nu", "pwsh",
];

pub(crate) fn is_shell_command(cmd: &str) -> bool {
    let normalized = cmd.strip_prefix('-').unwrap_or(cmd);
    let basename = normalized.rsplit('/').next().unwrap_or(normalized);
    KNOWN_SHELLS.contains(&basename)
}

pub fn is_pane_running_shell(session_name: &str) -> bool {
    pane_current_command(session_name)
        .map(|cmd| is_shell_command(&cmd))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serial_test::serial;
    use tempfile::TempDir;

    fn setup_test_home(temp: &TempDir) {
        std::env::set_var("HOME", temp.path());
        #[cfg(target_os = "linux")]
        std::env::set_var("XDG_CONFIG_HOME", temp.path().join(".config"));
    }

    fn instance_with_created_at(
        title: &str,
        path: &str,
        created_at: chrono::DateTime<Utc>,
    ) -> Instance {
        let mut instance = Instance::new(title, path);
        instance.created_at = created_at;
        instance
    }

    #[test]
    fn test_sanitize_session_name() {
        assert_eq!(sanitize_session_name("my-project"), "my-project");
        assert_eq!(sanitize_session_name("my project"), "my_project");
        assert_eq!(sanitize_session_name("a".repeat(30).as_str()).len(), 20);
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[32mgreen\x1b[0m"), "green");
        assert_eq!(strip_ansi("no codes here"), "no codes here");
        assert_eq!(strip_ansi("\x1b[1;34mbold blue\x1b[0m"), "bold blue");
    }

    #[test]
    fn test_strip_ansi_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn test_strip_ansi_multiple_codes() {
        assert_eq!(
            strip_ansi("\x1b[1m\x1b[32mbold green\x1b[0m normal"),
            "bold green normal"
        );
    }

    #[test]
    fn test_strip_ansi_osc_sequences() {
        assert_eq!(strip_ansi("\x1b]0;Window Title\x07text"), "text");
    }

    #[test]
    fn test_strip_ansi_nested_sequences() {
        assert_eq!(strip_ansi("\x1b[38;5;196mred\x1b[0m"), "red");
    }

    #[test]
    fn test_strip_ansi_with_256_colors() {
        assert_eq!(
            strip_ansi("\x1b[38;2;255;100;50mRGB color\x1b[0m"),
            "RGB color"
        );
    }

    #[test]
    fn test_sanitize_session_name_special_chars() {
        assert_eq!(sanitize_session_name("test/path"), "test_path");
        assert_eq!(sanitize_session_name("test.name"), "test_name");
        assert_eq!(sanitize_session_name("test@name"), "test_name");
        assert_eq!(sanitize_session_name("test:name"), "test_name");
    }

    #[test]
    fn test_sanitize_session_name_preserves_valid_chars() {
        assert_eq!(sanitize_session_name("test-name_123"), "test-name_123");
    }

    #[test]
    fn test_sanitize_session_name_empty() {
        assert_eq!(sanitize_session_name(""), "");
    }

    #[test]
    fn test_sanitize_session_name_unicode() {
        let result = sanitize_session_name("test😀emoji");
        assert!(result.starts_with("test"));
        assert!(result.contains('_'));
        assert!(!result.contains('😀'));
    }

    #[test]
    fn test_is_shell_command_recognizes_common_shells() {
        for shell in KNOWN_SHELLS {
            assert!(
                is_shell_command(shell),
                "{shell} should be recognized as a shell"
            );
        }
    }

    #[test]
    fn test_is_shell_command_recognizes_login_shells() {
        for shell in ["-bash", "-zsh", "-sh", "-fish"] {
            assert!(
                is_shell_command(shell),
                "{shell} should be recognized as a login shell"
            );
        }
    }

    #[test]
    fn test_is_shell_command_rejects_agent_binaries() {
        for cmd in [
            "claude", "opencode", "codex", "gemini", "cursor", "sleep", "python",
        ] {
            assert!(
                !is_shell_command(cmd),
                "{cmd} should not be recognized as a shell"
            );
        }
    }

    #[test]
    fn test_client_context_option_key_sanitizes_client_name() {
        assert_eq!(
            client_context_option_key(AOE_RETURN_SESSION_OPTION_PREFIX, "/dev/ttys012"),
            "@aoe_return_session__dev_ttys012"
        );
        assert_eq!(
            client_context_option_key(AOE_LAST_DETACHED_SESSION_OPTION_PREFIX, "/dev/ttys012"),
            "@aoe_last_detached_session__dev_ttys012"
        );
    }

    #[test]
    fn test_parse_session_name_for_client_matches_requested_client() {
        let stdout = "/dev/ttys008\tmonkeys\n/dev/ttys029\taoe_skills-manager-shell_cd9e9d61\n";
        assert_eq!(
            parse_session_name_for_client(stdout, "/dev/ttys029"),
            Some("aoe_skills-manager-shell_cd9e9d61".to_string())
        );
        assert_eq!(parse_session_name_for_client(stdout, "/dev/ttys999"), None);
    }

    #[test]
    fn test_instance_for_tmux_session_name_matches_terminal_variants() {
        let instance = instance_with_created_at("Skills Manager", "/tmp/skills", Utc::now());
        let instances = vec![instance.clone()];

        let agent_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
        let terminal_name =
            crate::tmux::TerminalSession::generate_name(&instance.id, &instance.title);
        let container_name =
            crate::tmux::ContainerTerminalSession::generate_name(&instance.id, &instance.title);

        assert_eq!(
            instance_for_tmux_session_name(&instances, &agent_name).map(|i| i.id.as_str()),
            Some(instance.id.as_str())
        );
        assert_eq!(
            instance_for_tmux_session_name(&instances, &terminal_name).map(|i| i.id.as_str()),
            Some(instance.id.as_str())
        );
        assert_eq!(
            instance_for_tmux_session_name(&instances, &container_name).map(|i| i.id.as_str()),
            Some(instance.id.as_str())
        );
    }

    #[test]
    fn test_detach_run_shell_cmd_uses_saved_return_target() {
        let cmd = detach_run_shell_cmd();
        assert!(cmd.contains("@aoe_last_detached_session_${client_key}"));
        assert!(cmd.contains("tmux set-option -gq"));
        assert!(cmd.contains("\"#{session_name}\""));
        assert!(cmd.contains("@aoe_return_session_${client_key}"));
        assert!(cmd.contains("tmux show-option -gv"));
        assert!(cmd.contains("switch-client -c \"$client_name\" -t \"$target\""));
        assert!(cmd.contains(
            "switch-client -c \"$client_name\" -l 2>/dev/null || tmux detach-client -t \"$client_name\""
        ));
    }

    #[test]
    fn test_cycle_run_shell_cmd_uses_saved_profile() {
        let cmd = cycle_run_shell_cmd("next");
        assert!(cmd.contains("@aoe_origin_profile_${client_key}"));
        assert!(cmd.contains("tmux show-option -gv"));
        assert!(cmd.contains(
            "tmux switch-session --direction next --profile \"$profile\" --client-name \"$client_name\""
        ));
    }

    #[test]
    fn test_resolve_cycle_target_wraps_forward() {
        let sessions = vec![
            "aoe_a".to_string(),
            "aoe_b".to_string(),
            "aoe_c".to_string(),
        ];
        assert_eq!(
            resolve_cycle_target(&sessions, "aoe_c", "next"),
            Some("aoe_a".to_string())
        );
    }

    #[test]
    fn test_resolve_cycle_target_requires_current_session_in_scope() {
        let sessions = vec!["aoe_a".to_string(), "aoe_b".to_string()];
        assert_eq!(resolve_cycle_target(&sessions, "aoe_other", "next"), None);
    }

    #[test]
    fn test_ordered_profile_sessions_for_cycle_matches_flattened_group_order() {
        let now = Utc::now();
        let ungrouped = instance_with_created_at("Ungrouped", "/tmp/u", now);
        let mut zebra = instance_with_created_at("Zebra", "/tmp/z", now + Duration::seconds(1));
        zebra.group_path = "work".to_string();
        let mut apple = instance_with_created_at("Apple", "/tmp/a", now + Duration::seconds(2));
        apple.group_path = "work".to_string();
        let instances = vec![ungrouped.clone(), zebra.clone(), apple.clone()];
        let groups = vec![Group {
            name: "work".to_string(),
            path: "work".to_string(),
            collapsed: false,
            children: Vec::new(),
        }];

        let sessions = ordered_profile_session_names(&instances, &groups, SortOrder::AZ);

        assert_eq!(
            sessions,
            vec![
                crate::tmux::Session::generate_name(&ungrouped.id, &ungrouped.title),
                crate::tmux::Session::generate_name(&apple.id, &apple.title),
                crate::tmux::Session::generate_name(&zebra.id, &zebra.title),
            ]
        );
    }

    #[test]
    fn test_ordered_profile_sessions_for_cycle_skips_collapsed_groups() {
        let now = Utc::now();
        let visible = instance_with_created_at("Visible", "/tmp/v", now);
        let mut hidden = instance_with_created_at("Hidden", "/tmp/h", now + Duration::seconds(1));
        hidden.group_path = "work".to_string();
        let instances = vec![visible.clone(), hidden.clone()];
        let groups = vec![Group {
            name: "work".to_string(),
            path: "work".to_string(),
            collapsed: true,
            children: Vec::new(),
        }];

        let sessions = ordered_profile_session_names(&instances, &groups, SortOrder::Newest);

        assert_eq!(
            sessions,
            vec![crate::tmux::Session::generate_name(
                &visible.id,
                &visible.title
            )]
        );
    }

    #[test]
    fn test_ordered_scoped_profile_session_names_limits_to_current_group() {
        let now = Utc::now();
        let mut group_alpha =
            instance_with_created_at("Alpha", "/tmp/alpha", now + Duration::seconds(1));
        group_alpha.group_path = "skills-manager".to_string();
        let mut group_beta =
            instance_with_created_at("Beta", "/tmp/beta", now + Duration::seconds(2));
        group_beta.group_path = "skills-manager".to_string();
        let mut other_group =
            instance_with_created_at("Gamma", "/tmp/gamma", now + Duration::seconds(3));
        other_group.group_path = "blog-workspace".to_string();
        let ungrouped = instance_with_created_at("Ungrouped", "/tmp/ungrouped", now);
        let instances = vec![
            ungrouped,
            group_alpha.clone(),
            group_beta.clone(),
            other_group,
        ];
        let groups = vec![
            Group {
                name: "skills-manager".to_string(),
                path: "skills-manager".to_string(),
                collapsed: false,
                children: Vec::new(),
            },
            Group {
                name: "blog-workspace".to_string(),
                path: "blog-workspace".to_string(),
                collapsed: false,
                children: Vec::new(),
            },
        ];
        let current = crate::tmux::Session::generate_name(&group_alpha.id, &group_alpha.title);

        let sessions =
            ordered_scoped_profile_session_names(&instances, &groups, SortOrder::AZ, &current);

        assert_eq!(
            sessions,
            vec![
                crate::tmux::Session::generate_name(&group_alpha.id, &group_alpha.title),
                crate::tmux::Session::generate_name(&group_beta.id, &group_beta.title),
            ]
        );
    }

    #[test]
    fn test_ordered_scoped_profile_session_names_limits_ungrouped_sessions() {
        let now = Utc::now();
        let alpha = instance_with_created_at("Alpha", "/tmp/alpha", now + Duration::seconds(1));
        let beta = instance_with_created_at("Beta", "/tmp/beta", now + Duration::seconds(2));
        let mut grouped =
            instance_with_created_at("Grouped", "/tmp/grouped", now + Duration::seconds(3));
        grouped.group_path = "skills-manager".to_string();
        let instances = vec![beta.clone(), grouped, alpha.clone()];
        let groups = vec![Group {
            name: "skills-manager".to_string(),
            path: "skills-manager".to_string(),
            collapsed: false,
            children: Vec::new(),
        }];
        let current = crate::tmux::Session::generate_name(&alpha.id, &alpha.title);

        let sessions =
            ordered_scoped_profile_session_names(&instances, &groups, SortOrder::AZ, &current);

        assert_eq!(
            sessions,
            vec![
                crate::tmux::Session::generate_name(&alpha.id, &alpha.title),
                crate::tmux::Session::generate_name(&beta.id, &beta.title),
            ]
        );
    }

    #[test]
    fn test_ordered_scoped_profile_session_names_requires_current_in_visible_order() {
        let now = Utc::now();
        let mut hidden_current =
            instance_with_created_at("Hidden", "/tmp/hidden", now + Duration::seconds(1));
        hidden_current.group_path = "skills-manager".to_string();
        let instances = vec![hidden_current.clone()];
        let groups = vec![Group {
            name: "skills-manager".to_string(),
            path: "skills-manager".to_string(),
            collapsed: true,
            children: Vec::new(),
        }];
        let current =
            crate::tmux::Session::generate_name(&hidden_current.id, &hidden_current.title);

        let sessions =
            ordered_scoped_profile_session_names(&instances, &groups, SortOrder::AZ, &current);

        assert!(sessions.is_empty());
    }

    #[test]
    #[serial]
    fn test_current_home_sort_order_reads_saved_app_state() {
        let temp = TempDir::new().unwrap();
        setup_test_home(&temp);
        let storage = crate::session::Storage::new("default").unwrap();
        let _ = storage.load_with_groups().unwrap();

        let mut config = crate::session::config::Config::default();
        config.app_state.sort_order = Some(SortOrder::ZA);
        crate::session::config::save_config(&config).unwrap();

        assert_eq!(current_home_sort_order(), SortOrder::ZA);
    }
}
