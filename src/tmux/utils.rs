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
///   `aoe tmux refresh-bindings` which sets d/n/p via `Command::new("tmux")`
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
        r##"if-shell -F "#{{m:aoe_*,#{{session_name}}}}" "run-shell '{aoe_bin} tmux refresh-bindings --client-name #{{client_name}}'" "bind-key d detach-client ; unbind-key n ; unbind-key p ; unbind-key N ; unbind-key P ; unbind-key h ; unbind-key j ; unbind-key k ; unbind-key l ; unbind-key 1 ; unbind-key 2 ; unbind-key 3 ; unbind-key 4 ; unbind-key 5 ; unbind-key 6 ; unbind-key 7 ; unbind-key 8 ; unbind-key 9""##,
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
    Command::new("tmux")
        .args(["unbind-key", "-T", "root", "C-q"])
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
    session_cycle_run_shell_cmds_with_scope(profile, false)
}

fn session_cycle_global_run_shell_cmds(profile: &str) -> (String, String) {
    session_cycle_run_shell_cmds_with_scope(profile, true)
}

fn session_cycle_run_shell_cmds_with_scope(profile: &str, global: bool) -> (String, String) {
    let aoe_bin = aoe_bin_path();
    let escaped = shell_escape(&aoe_bin);
    let escaped_profile = shell_escape(profile);
    let global_flag = if global { " --global" } else { "" };
    let next = format!(
        "client_name=\"#{{client_name}}\"; {escaped} tmux switch-session --direction next{global_flag} --profile {escaped_profile} --client-name \"$client_name\""
    );
    let prev = format!(
        "client_name=\"#{{client_name}}\"; {escaped} tmux switch-session --direction prev{global_flag} --profile {escaped_profile} --client-name \"$client_name\""
    );
    (next, prev)
}

fn index_jump_run_shell_cmd(index: usize, profile: &str) -> String {
    let aoe_bin = shell_escape(&aoe_bin_path());
    let escaped_profile = shell_escape(profile);
    format!(
        "client_name=\"#{{client_name}}\"; {aoe_bin} tmux switch-session --index {index} --profile {escaped_profile} --client-name \"$client_name\""
    )
}

fn index_jump_run_shell_cmd_from_option(index: usize) -> String {
    let aoe_bin = shell_escape(&aoe_bin_path());
    format!(
        concat!(
            "client_name=\"#{{client_name}}\"; ",
            "client_key=$(printf '%s' \"$client_name\" | tr -c '[:alnum:]' '_'); ",
            "profile=$(tmux show-option -gv \"{}${{client_key}}\" 2>/dev/null); ",
            "if [ -n \"$profile\" ]; then ",
            "{} tmux switch-session --index {} --profile \"$profile\" --client-name \"$client_name\"; ",
            "fi"
        ),
        AOE_ORIGIN_PROFILE_OPTION_PREFIX,
        aoe_bin,
        index
    )
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

fn cycle_run_shell_cmd(direction: &str, global: bool) -> String {
    let aoe_bin = shell_escape(&aoe_bin_path());
    let global_flag = if global { " --global" } else { "" };
    format!(
        concat!(
            "client_name=\"#{{client_name}}\"; ",
            "client_key=$(printf '%s' \"$client_name\" | tr -c '[:alnum:]' '_'); ",
            "profile=$(tmux show-option -gv \"{}${{client_key}}\" 2>/dev/null); ",
            "if [ -n \"$profile\" ]; then ",
            "{} tmux switch-session --direction {}{} --profile \"$profile\" --client-name \"$client_name\"; ",
            "fi"
        ),
        AOE_ORIGIN_PROFILE_OPTION_PREFIX,
        aoe_bin,
        direction,
        global_flag
    )
}

fn apply_managed_session_bindings(client_name: Option<&str>) {
    let detach_cmd = detach_run_shell_cmd();
    let next_cmd = cycle_run_shell_cmd("next", false);
    let prev_cmd = cycle_run_shell_cmd("prev", false);
    let global_next_cmd = cycle_run_shell_cmd("next", true);
    let global_prev_cmd = cycle_run_shell_cmd("prev", true);
    Command::new("tmux")
        .args(["bind-key", "d", "run-shell", &detach_cmd])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "n", "run-shell", &next_cmd])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "p", "run-shell", &prev_cmd])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "N", "run-shell", &global_next_cmd])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "P", "run-shell", &global_prev_cmd])
        .output()
        .ok();
    // Ctrl+q in root table: detach if in aoe_* session, otherwise pass through.
    let root_ctrl_q_cmd = root_ctrl_q_run_shell_cmd();
    Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "root",
            "C-q",
            "run-shell",
            &root_ctrl_q_cmd,
        ])
        .output()
        .ok();

    // Override number jump bindings with profile-from-option variants (nested mode)
    for first_digit in 1..=9u8 {
        let table_name = format!("aoe-{}", first_digit);

        Command::new("tmux")
            .args([
                "bind-key",
                &first_digit.to_string(),
                "switch-client",
                "-T",
                &table_name,
            ])
            .output()
            .ok();

        let single_cmd = index_jump_run_shell_cmd_from_option(first_digit as usize);
        Command::new("tmux")
            .args([
                "bind-key",
                "-T",
                &table_name,
                "Space",
                "run-shell",
                &single_cmd,
            ])
            .output()
            .ok();

        for second_digit in 0..=9u8 {
            let two_digit_index = (first_digit as usize) * 10 + (second_digit as usize);
            let two_digit_cmd = index_jump_run_shell_cmd_from_option(two_digit_index);
            Command::new("tmux")
                .args([
                    "bind-key",
                    "-T",
                    &table_name,
                    &second_digit.to_string(),
                    "run-shell",
                    &two_digit_cmd,
                ])
                .output()
                .ok();
        }
    }

    let _ = client_name;
}

fn root_ctrl_q_run_shell_cmd() -> String {
    let detach = detach_run_shell_cmd();
    format!(
        "session=\"#{{session_name}}\"; case \"$session\" in aoe_*) {} ;; *) tmux send-keys C-q ;; esac",
        detach
    )
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
        Command::new("tmux")
            .args(["unbind-key", "-T", "root", "C-q"])
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

/// Binds `Ctrl+b n` / `Ctrl+b p` to cycle through aoe agent sessions
/// belonging to the given profile. Works in both nested and non-nested tmux modes.
pub fn setup_session_cycle_bindings(profile: &str) {
    tag_sessions_with_profile(profile);

    let (switch_next, switch_prev) = session_cycle_run_shell_cmds(profile);
    let (switch_global_next, switch_global_prev) = session_cycle_global_run_shell_cmds(profile);
    Command::new("tmux")
        .args(["bind-key", "n", "run-shell", &switch_next])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "p", "run-shell", &switch_prev])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "N", "run-shell", &switch_global_next])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "P", "run-shell", &switch_global_prev])
        .output()
        .ok();
    for (key, dir) in [("h", "-L"), ("j", "-D"), ("k", "-U"), ("l", "-R")] {
        Command::new("tmux")
            .args(["bind-key", key, "select-pane", dir])
            .output()
            .ok();
    }
    // Ctrl+q in root table: detach if in aoe_* session, pass through otherwise.
    // In nested mode, apply_managed_session_bindings() overwrites this with the
    // more sophisticated switch-client command.
    Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "root",
            "C-q",
            "run-shell",
            "session=\"#{session_name}\"; case \"$session\" in aoe_*) tmux detach-client ;; *) tmux send-keys C-q ;; esac",
        ])
        .output()
        .ok();

    // Number jump: Ctrl+b 1-9 enters aoe-N key tables for two-phase digit input
    setup_number_jump_bindings(profile);
}

fn setup_number_jump_bindings(profile: &str) {
    for first_digit in 1..=9u8 {
        let table_name = format!("aoe-{}", first_digit);

        // Bind Ctrl+b <digit> -> switch to aoe-N key table
        Command::new("tmux")
            .args([
                "bind-key",
                &first_digit.to_string(),
                "switch-client",
                "-T",
                &table_name,
            ])
            .output()
            .ok();

        // In aoe-N table: Space confirms single-digit jump
        let single_cmd = index_jump_run_shell_cmd(first_digit as usize, profile);
        Command::new("tmux")
            .args([
                "bind-key",
                "-T",
                &table_name,
                "Space",
                "run-shell",
                &single_cmd,
            ])
            .output()
            .ok();

        // In aoe-N table: 0-9 auto-confirms two-digit jump
        for second_digit in 0..=9u8 {
            let two_digit_index = (first_digit as usize) * 10 + (second_digit as usize);
            let two_digit_cmd = index_jump_run_shell_cmd(two_digit_index, profile);
            Command::new("tmux")
                .args([
                    "bind-key",
                    "-T",
                    &table_name,
                    &second_digit.to_string(),
                    "run-shell",
                    &two_digit_cmd,
                ])
                .output()
                .ok();
        }
    }
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
    for key in ["n", "p", "N", "P", "h", "j", "k", "l"] {
        Command::new("tmux").args(["unbind-key", key]).output().ok();
    }
    Command::new("tmux")
        .args(["unbind-key", "-T", "root", "C-q"])
        .output()
        .ok();
    cleanup_number_jump_bindings();
}

fn cleanup_number_jump_bindings() {
    for digit in 1..=9u8 {
        Command::new("tmux")
            .args(["unbind-key", &digit.to_string()])
            .output()
            .ok();

        let table_name = format!("aoe-{}", digit);
        Command::new("tmux")
            .args(["unbind-key", "-T", &table_name, "Space"])
            .output()
            .ok();
        for second in 0..=9u8 {
            Command::new("tmux")
                .args(["unbind-key", "-T", &table_name, &second.to_string()])
                .output()
                .ok();
        }
    }
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
    global: bool,
    profile: &str,
    client_name: Option<&str>,
) -> anyhow::Result<()> {
    let storage = crate::session::Storage::new(profile)?;
    let (instances, groups) = storage.load_with_groups()?;
    let Some(current) = current_tmux_session_name(client_name)? else {
        return Ok(());
    };
    let sessions = if global {
        ordered_global_profile_sessions_for_cycle(&instances, &groups, current_home_sort_order())
    } else {
        ordered_profile_sessions_for_cycle(&instances, &groups, current_home_sort_order(), &current)
    };

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

fn ordered_global_profile_sessions_for_cycle(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
) -> Vec<String> {
    let expanded_groups = expanded_groups(groups);
    ordered_profile_session_names(instances, &expanded_groups, sort_order)
        .into_iter()
        .filter(|name| tmux_session_exists(name))
        .collect()
}

fn expanded_groups(groups: &[Group]) -> Vec<Group> {
    groups
        .iter()
        .cloned()
        .map(|mut group| {
            group.collapsed = false;
            group.children.clear();
            group
        })
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

pub fn switch_aoe_session_by_index(
    index: usize,
    profile: &str,
    client_name: Option<&str>,
) -> anyhow::Result<()> {
    if index == 0 {
        return Ok(());
    }

    let storage = crate::session::Storage::new(profile)?;
    let (instances, groups) = storage.load_with_groups()?;
    let sessions = ordered_profile_session_names(&instances, &groups, current_home_sort_order());

    let existing: Vec<String> = sessions
        .into_iter()
        .filter(|name| tmux_session_exists(name))
        .collect();

    let Some(target_session) = existing.get(index - 1) else {
        return Ok(());
    };

    if let Some(client_name) = client_name {
        set_last_detached_session_for_client(client_name, target_session);
    }

    switch_client_to_session(target_session, client_name)?;
    Ok(())
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

#[cfg(test)]
pub(crate) fn sanitize_session_name(name: &str) -> String {
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
    if trimmed.is_empty() || trimmed.contains('#') {
        // Discard unexpanded format specifiers (e.g. "#{pane_id}") from sessions
        // created before the -F flag fix
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn append_store_pane_id_args(args: &mut Vec<String>, target: &str) {
    // -F enables format expansion so #{pane_id} resolves to the actual pane ID (e.g. %42)
    args.extend([
        ";".to_string(),
        "set-option".to_string(),
        "-F".to_string(),
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
    fn test_instance_for_tmux_session_name_matches_agent_session() {
        let instance = instance_with_created_at("Skills Manager", "/tmp/skills", Utc::now());
        let instances = vec![instance.clone()];

        let agent_name = crate::tmux::Session::generate_name(&instance.id, &instance.title);

        assert_eq!(
            instance_for_tmux_session_name(&instances, &agent_name).map(|i| i.id.as_str()),
            Some(instance.id.as_str())
        );
        assert!(instance_for_tmux_session_name(&instances, "nonexistent_session").is_none());
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
    fn test_root_ctrl_q_cmd_guards_on_session_name() {
        let cmd = root_ctrl_q_run_shell_cmd();
        assert!(cmd.contains("case \"$session\" in aoe_*)"));
        assert!(cmd.contains("send-keys C-q"));
        assert!(cmd.contains("@aoe_return_session_${client_key}"));
    }

    #[test]
    fn test_cycle_run_shell_cmd_uses_saved_profile() {
        let cmd = cycle_run_shell_cmd("next", false);
        assert!(cmd.contains("@aoe_origin_profile_${client_key}"));
        assert!(cmd.contains("tmux show-option -gv"));
        assert!(cmd.contains(
            "tmux switch-session --direction next --profile \"$profile\" --client-name \"$client_name\""
        ));
    }

    #[test]
    fn test_cycle_run_shell_cmd_adds_global_flag_when_requested() {
        let cmd = cycle_run_shell_cmd("prev", true);
        assert!(cmd.contains("@aoe_origin_profile_${client_key}"));
        assert!(cmd.contains(
            "tmux switch-session --direction prev --global --profile \"$profile\" --client-name \"$client_name\""
        ));
    }

    #[test]
    fn test_session_cycle_global_run_shell_cmds_include_global_flag() {
        let (next, prev) = session_cycle_global_run_shell_cmds("default");
        assert!(next.contains("--direction next --global"));
        assert!(prev.contains("--direction prev --global"));
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
            default_directory: None,
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
            default_directory: None,
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
    fn test_ordered_global_profile_sessions_for_cycle_ignores_collapsed_groups() {
        let now = Utc::now();
        let visible = instance_with_created_at("Visible", "/tmp/v", now);
        let mut hidden = instance_with_created_at("Hidden", "/tmp/h", now + Duration::seconds(1));
        hidden.group_path = "work".to_string();
        let instances = vec![visible.clone(), hidden.clone()];
        let groups = vec![Group {
            name: "work".to_string(),
            path: "work".to_string(),
            collapsed: true,
            default_directory: None,
            children: Vec::new(),
        }];

        let sessions =
            ordered_profile_session_names(&instances, &expanded_groups(&groups), SortOrder::Newest);

        assert_eq!(
            sessions,
            vec![
                crate::tmux::Session::generate_name(&visible.id, &visible.title),
                crate::tmux::Session::generate_name(&hidden.id, &hidden.title),
            ]
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
                default_directory: None,
                children: Vec::new(),
            },
            Group {
                name: "blog-workspace".to_string(),
                path: "blog-workspace".to_string(),
                collapsed: false,
                default_directory: None,
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
            default_directory: None,
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
            default_directory: None,
            children: Vec::new(),
        }];
        let current =
            crate::tmux::Session::generate_name(&hidden_current.id, &hidden_current.title);

        let sessions =
            ordered_scoped_profile_session_names(&instances, &groups, SortOrder::AZ, &current);

        assert!(sessions.is_empty());
    }

    #[test]
    fn test_index_resolution_returns_correct_session() {
        let now = Utc::now();
        let a = instance_with_created_at("Alpha", "/tmp/a", now);
        let b = instance_with_created_at("Beta", "/tmp/b", now + Duration::seconds(1));
        let c = instance_with_created_at("Charlie", "/tmp/c", now + Duration::seconds(2));
        let instances = vec![a.clone(), b.clone(), c.clone()];
        let groups = vec![];

        let sessions = ordered_profile_session_names(&instances, &groups, SortOrder::AZ);

        assert_eq!(sessions.len(), 3);
        assert_eq!(
            sessions.first(),
            Some(&crate::tmux::Session::generate_name(&a.id, &a.title))
        );
        assert_eq!(
            sessions.get(1),
            Some(&crate::tmux::Session::generate_name(&b.id, &b.title))
        );
        assert_eq!(
            sessions.get(2),
            Some(&crate::tmux::Session::generate_name(&c.id, &c.title))
        );
        // Out of range returns None
        assert_eq!(sessions.get(3), None);
    }

    #[test]
    fn test_index_resolution_zero_is_invalid() {
        let sessions = vec!["aoe_a".to_string(), "aoe_b".to_string()];
        // Index 0 should not match any session (1-based indexing)
        assert_eq!(sessions.get(0_usize.wrapping_sub(1)), None);
    }

    #[test]
    fn test_index_resolution_with_groups_skips_group_headers() {
        let now = Utc::now();
        let a = instance_with_created_at("Alpha", "/tmp/a", now);
        let mut b = instance_with_created_at("Beta", "/tmp/b", now + Duration::seconds(1));
        b.group_path = "work".to_string();
        let mut c = instance_with_created_at("Charlie", "/tmp/c", now + Duration::seconds(2));
        c.group_path = "work".to_string();
        let instances = vec![a.clone(), b.clone(), c.clone()];
        let groups = vec![Group {
            name: "work".to_string(),
            path: "work".to_string(),
            collapsed: false,
            default_directory: None,
            children: Vec::new(),
        }];

        let sessions = ordered_profile_session_names(&instances, &groups, SortOrder::AZ);

        // All 3 sessions should be present (groups are filtered out)
        assert_eq!(sessions.len(), 3);
        // Index 1 = Alpha (ungrouped, appears first in AZ)
        assert_eq!(
            sessions.first(),
            Some(&crate::tmux::Session::generate_name(&a.id, &a.title))
        );
        // Index 2 = Beta (in work group)
        assert_eq!(
            sessions.get(1),
            Some(&crate::tmux::Session::generate_name(&b.id, &b.title))
        );
    }

    #[test]
    fn test_index_jump_run_shell_cmd_contains_index() {
        let cmd = index_jump_run_shell_cmd(5, "default");
        assert!(cmd.contains("--index 5"));
        assert!(cmd.contains("--profile"));
    }

    #[test]
    fn test_index_jump_run_shell_cmd_from_option_contains_index() {
        let cmd = index_jump_run_shell_cmd_from_option(13);
        assert!(cmd.contains("--index 13"));
        assert!(cmd.contains("@aoe_origin_profile_"));
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
