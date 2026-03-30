//! tmux utility functions

use std::process::Command;

use crate::session::{
    config::{load_config, SortOrder},
    expanded_groups, flatten_tree, Group, GroupTree, Instance, Item,
};

const AOE_PROFILE_OPTION: &str = "@aoe_profile";
const AOE_LAST_DETACHED_SESSION_OPTION_PREFIX: &str = "@aoe_last_detached_session_";
const AOE_PREV_SESSION_OPTION_PREFIX: &str = "@aoe_prev_session_";
const AOE_INDEX_OPTION: &str = "@aoe_index";
const AOE_TITLE_OPTION: &str = "@aoe_title";
const AOE_FROM_TITLE_OPTION: &str = "@aoe_from_title";
const CTRL_COMMA_CSI_U_PASSTHROUGH: &str = "send-keys -H 1b 5b 34 34 3b 35 75";
const CTRL_PERIOD_CSI_U_PASSTHROUGH: &str = "send-keys -H 1b 5b 34 36 3b 35 75";

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

fn index_jump_run_shell_cmd(index: usize, profile: &str) -> String {
    let aoe_bin = shell_escape(&aoe_bin_path());
    let escaped_profile = shell_escape(profile);
    format!(
        "client_name=\"#{{client_name}}\"; {aoe_bin} tmux switch-session --index {index} --profile {escaped_profile} --client-name \"$client_name\""
    )
}

fn back_toggle_run_shell_cmd(profile: &str) -> String {
    let aoe_bin = shell_escape(&aoe_bin_path());
    let escaped_profile = shell_escape(profile);
    format!(
        "client_name=\"#{{client_name}}\"; {aoe_bin} tmux switch-session --back --profile {escaped_profile} --client-name \"$client_name\""
    )
}

fn root_ctrl_q_run_shell_cmd() -> String {
    "session=\"#{session_name}\"; case \"$session\" in aoe_*) tmux detach-client ;; *) tmux send-keys C-q ;; esac"
        .to_string()
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

/// Consume the last managed session visited by a client so the TUI can restore
/// selection to the session that was most recently detached.
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

fn resolve_client_name(client_name: Option<&str>) -> Option<String> {
    client_name
        .map(str::to_owned)
        .or_else(crate::tmux::get_current_client_name)
}

fn set_global_option(option_key: &str, value: &str) {
    Command::new("tmux")
        .args(["set-option", "-gq"])
        .arg(option_key)
        .arg(value)
        .output()
        .ok();
}

fn unset_global_option(option_key: &str) {
    Command::new("tmux")
        .args(["set-option", "-gqu"])
        .arg(option_key)
        .output()
        .ok();
}

fn get_global_option(option_key: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["show-option", "-gv"])
        .arg(option_key)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn set_tmux_session_option(session_name: &str, option: &str, value: &str) {
    Command::new("tmux")
        .args(["set-option", "-t", session_name, option, value])
        .output()
        .ok();
}

fn unset_tmux_session_option(session_name: &str, option: &str) {
    Command::new("tmux")
        .args(["set-option", "-t", session_name, "-u", option])
        .output()
        .ok();
}

fn get_tmux_session_option(session_name: &str, option: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["show-options", "-t", session_name, "-v", option])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

pub fn set_previous_session_for_client(client_name: &str, session_name: &str) {
    let option_key = client_context_option_key(AOE_PREV_SESSION_OPTION_PREFIX, client_name);
    set_global_option(&option_key, session_name);
}

fn get_previous_session_for_client(client_name: &str) -> Option<String> {
    let option_key = client_context_option_key(AOE_PREV_SESSION_OPTION_PREFIX, client_name);
    get_global_option(&option_key)
}

pub fn clear_from_title(session_name: &str) {
    unset_tmux_session_option(session_name, AOE_FROM_TITLE_OPTION);
}

pub fn clear_previous_session_for_client(client_name: &str) {
    let option_key = client_context_option_key(AOE_PREV_SESSION_OPTION_PREFIX, client_name);
    unset_global_option(&option_key);
}

pub fn set_target_from_title(source_session: &str, target_session: &str) {
    if let Some(source_title) = get_tmux_session_option(source_session, AOE_TITLE_OPTION) {
        set_tmux_session_option(target_session, AOE_FROM_TITLE_OPTION, &source_title);
    } else {
        unset_tmux_session_option(target_session, AOE_FROM_TITLE_OPTION);
    }
}

fn session_index_in_order(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
    target_session: &str,
) -> Option<usize> {
    ordered_profile_session_names(instances, groups, sort_order)
        .into_iter()
        .position(|session| session == target_session)
        .map(|index| index + 1)
}

fn set_target_session_index(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
    target_session: &str,
) {
    if let Some(index) = session_index_in_order(instances, groups, sort_order, target_session) {
        set_tmux_session_option(target_session, AOE_INDEX_OPTION, &index.to_string());
    } else {
        unset_tmux_session_option(target_session, AOE_INDEX_OPTION);
    }
}

/// Set `@aoe_index` on a session so the status bar shows its number immediately.
/// Called from the TUI attach path so the index is visible on first entry,
/// not only after session cycling.
pub fn update_session_index(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
    target_session: &str,
) {
    set_target_session_index(instances, groups, sort_order, target_session);
}

fn track_session_switch(
    current_session: &str,
    target_session: &str,
    client_name: Option<&str>,
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
) {
    if let Some(client_name) = resolve_client_name(client_name).as_deref() {
        set_previous_session_for_client(client_name, current_session);
    }

    set_target_from_title(current_session, target_session);
    set_target_session_index(instances, groups, sort_order, target_session);
}

fn resolve_instance_id_for_session(target_session: &str, instances: &[Instance]) -> Option<String> {
    instances.iter().find_map(|instance| {
        (crate::tmux::Session::generate_name(&instance.id, &instance.title) == target_session)
            .then(|| instance.id.clone())
    })
}

fn acknowledge_switched_session(target_session: &str, instances: &[Instance]) {
    let Some(instance_id) = resolve_instance_id_for_session(target_session, instances) else {
        return;
    };

    if let Err(err) = super::write_ack_signal(&instance_id) {
        tracing::debug!(
            "Failed to write ack signal for session {} (instance {}): {}",
            target_session,
            instance_id,
            err
        );
    }
}

fn switch_to_previous_session<FExists, FSwitch>(
    current_session: Option<&str>,
    previous_session: Option<&str>,
    session_exists: FExists,
    switch_client: FSwitch,
) -> anyhow::Result<Option<String>>
where
    FExists: Fn(&str) -> bool,
    FSwitch: FnOnce(&str) -> anyhow::Result<()>,
{
    let Some(current_session) = current_session else {
        return Ok(None);
    };
    let Some(previous_session) = previous_session.filter(|session| !session.is_empty()) else {
        return Ok(None);
    };

    if previous_session == current_session || !session_exists(previous_session) {
        return Ok(None);
    }

    switch_client(previous_session)?;
    Ok(Some(previous_session.to_string()))
}

/// Binds root-table session cycling plus prefix-table back toggle and number
/// jump keys for AoE-managed tmux sessions.
pub fn setup_session_cycle_bindings(profile: &str) {
    let mut lines = Vec::new();

    // Profile tagging
    collect_tag_sessions_with_profile(profile, &mut lines);

    // Base bindings
    let switch_back = back_toggle_run_shell_cmd(profile);
    let (switch_next, switch_prev) = session_cycle_run_shell_cmds(profile);
    let guarded_next = format!("run-shell {}", shell_escape(&switch_next));
    let guarded_prev = format!("run-shell {}", shell_escape(&switch_prev));

    lines.push(format!(
        "bind-key b run-shell {}",
        shell_escape(&switch_back)
    ));
    lines.push(format!(
        "bind-key -T root C-. if-shell -F '#{{m:aoe_*,#{{session_name}}}}' {} {}",
        shell_escape(&guarded_next),
        shell_escape(CTRL_PERIOD_CSI_U_PASSTHROUGH)
    ));
    lines.push(format!(
        "bind-key -T root C-, if-shell -F '#{{m:aoe_*,#{{session_name}}}}' {} {}",
        shell_escape(&guarded_prev),
        shell_escape(CTRL_COMMA_CSI_U_PASSTHROUGH)
    ));
    for (key, dir) in [("h", "-L"), ("j", "-D"), ("k", "-U"), ("l", "-R")] {
        lines.push(format!("bind-key {} select-pane {}", key, dir));
    }
    lines.push("bind-key -T root C-\\; select-pane -t :.+".to_string());
    lines.push(format!(
        "bind-key -T root C-q run-shell {}",
        shell_escape(&root_ctrl_q_run_shell_cmd())
    ));
    lines.push(
        "bind-key % if-shell -F '#{m:aoe_*,#{session_name}}' \"split-window -h -c '#{@aoe_project_path}'\" \"split-window -h\""
            .to_string(),
    );
    lines.push(
        "bind-key '\"' if-shell -F '#{m:aoe_*,#{session_name}}' \"split-window -v -c '#{@aoe_project_path}'\" \"split-window -v\""
            .to_string(),
    );

    // Number jump key tables
    collect_number_jump_bindings(profile, &mut lines);

    source_file_batch(&lines);
}

fn collect_number_jump_bindings(profile: &str, lines: &mut Vec<String>) {
    for first_digit in 1..=9u8 {
        let table_name = format!("aoe-{}", first_digit);

        lines.push(format!(
            "bind-key {} switch-client -T {}",
            first_digit, table_name
        ));

        let single_cmd = index_jump_run_shell_cmd(first_digit as usize, profile);
        lines.push(format!(
            "bind-key -T {} Space run-shell {}",
            table_name,
            shell_escape(&single_cmd)
        ));

        for second_digit in 0..=9u8 {
            let two_digit_index = (first_digit as usize) * 10 + (second_digit as usize);
            let two_digit_cmd = index_jump_run_shell_cmd(two_digit_index, profile);
            lines.push(format!(
                "bind-key -T {} {} run-shell {}",
                table_name,
                second_digit,
                shell_escape(&two_digit_cmd)
            ));
        }
    }
}

fn collect_tag_sessions_with_profile(profile: &str, lines: &mut Vec<String>) {
    let Ok(storage) = crate::session::Storage::new(profile) else {
        return;
    };
    let Ok(instances) = storage.load() else {
        return;
    };
    for instance in &instances {
        let name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
        lines.push(format!(
            "set-option -qt {} {} {}",
            shell_escape(&name),
            AOE_PROFILE_OPTION,
            shell_escape(profile)
        ));
        lines.push(format!(
            "set-option -qt {} @aoe_project_path {}",
            shell_escape(&name),
            shell_escape(&instance.project_path)
        ));
    }
}

/// Write all tmux commands to a temp file and execute via `tmux source-file`.
fn source_file_batch(lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let content = lines.join("\n");
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!("aoe-tmux-{}.conf", std::process::id()));
    if let Err(e) = std::fs::write(&tmp_path, &content) {
        tracing::warn!("Failed to write tmux commands to temp file: {}", e);
        return;
    }
    let output = Command::new("tmux")
        .args(["source-file"])
        .arg(&tmp_path)
        .output();
    let _ = std::fs::remove_file(&tmp_path);
    match output {
        Ok(o) if !o.status.success() => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            tracing::warn!("tmux source-file failed: {}", stderr.trim());
        }
        Err(e) => {
            tracing::warn!("Failed to run tmux source-file: {}", e);
        }
        _ => {}
    }
}

pub fn cleanup_session_cycle_bindings() {
    let mut lines = Vec::new();
    for key in ["b", "h", "j", "k", "l"] {
        lines.push(format!("unbind-key {}", key));
    }
    for key in ["C-\\;", "C-q", "C-,", "C-."] {
        lines.push(format!("unbind-key -T root {}", key));
    }
    for digit in 1..=9u8 {
        lines.push(format!("unbind-key {}", digit));
        let table_name = format!("aoe-{}", digit);
        lines.push(format!("unbind-key -T {} Space", table_name));
        for second in 0..=9u8 {
            lines.push(format!("unbind-key -T {} {}", table_name, second));
        }
    }
    source_file_batch(&lines);
}

pub(crate) fn shell_escape(s: &str) -> String {
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
    let resolved_client_name = resolve_client_name(client_name);
    let Some(current) = current_tmux_session_name(resolved_client_name.as_deref())? else {
        return Ok(());
    };
    let sort_order = current_home_sort_order();
    let sessions = ordered_global_profile_sessions_for_cycle(&instances, &groups, sort_order);

    if sessions.len() <= 1 {
        return Ok(());
    }

    let Some(target_session) = resolve_cycle_target(&sessions, &current, direction) else {
        return Ok(());
    };

    if let Some(client_name) = resolved_client_name.as_deref() {
        // Remember the actual managed session the user cycled to so TUI re-entry
        // can restore selection there without rewriting the AoE return target.
        set_last_detached_session_for_client(client_name, &target_session);
    }

    switch_client_to_session(&target_session, resolved_client_name.as_deref())?;
    acknowledge_switched_session(&target_session, &instances);
    track_session_switch(
        &current,
        &target_session,
        resolved_client_name.as_deref(),
        &instances,
        &groups,
        sort_order,
    );

    Ok(())
}

fn current_home_sort_order() -> SortOrder {
    load_config()
        .ok()
        .flatten()
        .and_then(|config| config.app_state.sort_order)
        .unwrap_or_default()
}

fn ordered_global_profile_sessions_for_cycle(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
) -> Vec<String> {
    ordered_profile_session_names(instances, groups, sort_order)
        .into_iter()
        .filter(|name| tmux_session_exists(name))
        .collect()
}

fn ordered_profile_session_names(
    instances: &[Instance],
    groups: &[Group],
    sort_order: SortOrder,
) -> Vec<String> {
    let group_tree = GroupTree::new_with_groups(instances, &expanded_groups(groups));
    flatten_tree(&group_tree, instances, sort_order)
        .into_iter()
        .filter_map(|item| match item {
            Item::Session { id, .. } => instances
                .iter()
                .find(|instance| instance.id == id)
                .map(|instance| crate::tmux::Session::generate_name(&instance.id, &instance.title)),
            Item::Group { .. } | Item::ProfileHeader { .. } => None,
        })
        .collect()
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
    let output = command.output()?;
    anyhow::ensure!(
        output.status.success(),
        "tmux switch-client failed for {}: {}",
        target_session,
        String::from_utf8_lossy(&output.stderr).trim()
    );
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

fn resolve_existing_session_by_index<FExists>(
    sessions: &[String],
    index: usize,
    session_exists: FExists,
) -> Option<String>
where
    FExists: Fn(&str) -> bool,
{
    let target_idx = index.checked_sub(1)?;
    let target_session = sessions.get(target_idx)?;
    session_exists(target_session).then(|| target_session.clone())
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
    let resolved_client_name = resolve_client_name(client_name);
    let current = current_tmux_session_name(resolved_client_name.as_deref())?;
    let sort_order = current_home_sort_order();
    let sessions = ordered_profile_session_names(&instances, &groups, sort_order);

    let Some(target_session) =
        resolve_existing_session_by_index(&sessions, index, tmux_session_exists)
    else {
        return Ok(());
    };

    if current.as_deref() == Some(target_session.as_str()) {
        return Ok(());
    }

    if let Some(client_name) = resolved_client_name.as_deref() {
        set_last_detached_session_for_client(client_name, &target_session);
    }

    switch_client_to_session(&target_session, resolved_client_name.as_deref())?;
    acknowledge_switched_session(&target_session, &instances);
    if let Some(current_session) = current.as_deref() {
        track_session_switch(
            current_session,
            &target_session,
            resolved_client_name.as_deref(),
            &instances,
            &groups,
            sort_order,
        );
    }
    Ok(())
}

pub fn switch_aoe_session_back(profile: &str, client_name: Option<&str>) -> anyhow::Result<()> {
    let resolved_client_name = resolve_client_name(client_name);
    let current = current_tmux_session_name(resolved_client_name.as_deref())?;
    let previous = resolved_client_name
        .as_deref()
        .and_then(get_previous_session_for_client);

    let Some(target_session) = switch_to_previous_session(
        current.as_deref(),
        previous.as_deref(),
        tmux_session_exists,
        |target_session| switch_client_to_session(target_session, resolved_client_name.as_deref()),
    )?
    else {
        return Ok(());
    };

    if let Some(client_name) = resolved_client_name.as_deref() {
        set_last_detached_session_for_client(client_name, &target_session);
    }

    let storage = crate::session::Storage::new(profile)?;
    let (instances, groups) = storage.load_with_groups()?;
    acknowledge_switched_session(&target_session, &instances);
    if let Some(current_session) = current.as_deref() {
        track_session_switch(
            current_session,
            &target_session,
            resolved_client_name.as_deref(),
            &instances,
            &groups,
            current_home_sort_order(),
        );
    }

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

pub fn append_store_project_path_args(args: &mut Vec<String>, target: &str, working_dir: &str) {
    args.extend([
        ";".to_string(),
        "set-option".to_string(),
        "-t".to_string(),
        target.to_string(),
        "@aoe_project_path".to_string(),
        working_dir.to_string(),
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
    use std::cell::{Cell, RefCell};
    use tempfile::TempDir;

    fn setup_test_home(temp: &TempDir) {
        std::env::set_var("HOME", temp.path());
        #[cfg(target_os = "linux")]
        std::env::set_var("XDG_CONFIG_HOME", temp.path().join(".config"));
    }

    fn tmux_available() -> bool {
        Command::new("tmux").arg("-V").output().is_ok()
    }

    fn create_tmux_session(session_name: &str) {
        let output = Command::new("tmux")
            .args(["new-session", "-d", "-s", session_name, "sh"])
            .output()
            .expect("tmux new-session");
        assert!(
            output.status.success(),
            "failed to create tmux session {}: {}",
            session_name,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn kill_tmux_session(session_name: &str) {
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", session_name])
            .output();
    }

    fn clear_global_option(option_key: &str) {
        let _ = Command::new("tmux")
            .args(["set-option", "-gu"])
            .arg(option_key)
            .output();
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
    fn test_append_store_project_path_args_appends_expected_sequence() {
        let mut args = vec!["new-session".to_string(), "-d".to_string()];

        append_store_project_path_args(&mut args, "aoe_demo", "/tmp/demo project");

        assert_eq!(
            args,
            vec![
                "new-session",
                "-d",
                ";",
                "set-option",
                "-t",
                "aoe_demo",
                "@aoe_project_path",
                "/tmp/demo project",
            ]
        );
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
            client_context_option_key(AOE_LAST_DETACHED_SESSION_OPTION_PREFIX, "/dev/ttys012"),
            "@aoe_last_detached_session__dev_ttys012"
        );
        assert_eq!(
            client_context_option_key(AOE_PREV_SESSION_OPTION_PREFIX, "/dev/ttys012"),
            "@aoe_prev_session__dev_ttys012"
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
    fn test_root_ctrl_q_cmd_guards_on_session_name() {
        let cmd = root_ctrl_q_run_shell_cmd();
        assert!(
            cmd.contains("#{session_name}"),
            "must use single-brace tmux format, not double-brace"
        );
        assert!(cmd.contains("case \"$session\" in aoe_*)"));
        assert!(cmd.contains("send-keys C-q"));
        assert!(cmd.contains("tmux detach-client"));
    }

    #[test]
    fn test_session_cycle_run_shell_cmds_do_not_use_global_flag() {
        let (next, prev) = session_cycle_run_shell_cmds("default");
        assert!(next.contains("--direction next --profile"));
        assert!(prev.contains("--direction prev --profile"));
        assert!(!next.contains("--global"));
        assert!(!prev.contains("--global"));
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
        let sessions = ["aoe_a".to_string(), "aoe_b".to_string()];
        assert_eq!(resolve_cycle_target(&sessions, "aoe_other", "next"), None);
    }

    #[test]
    fn test_resolve_instance_id_for_session_matches_generated_name() {
        let alpha = Instance::new("Alpha", "/tmp/alpha");
        let beta = Instance::new("Beta", "/tmp/beta");
        let beta_session = crate::tmux::Session::generate_name(&beta.id, &beta.title);

        assert_eq!(
            resolve_instance_id_for_session(&beta_session, &[alpha, beta.clone()]),
            Some(beta.id)
        );
    }

    #[test]
    fn test_resolve_instance_id_for_session_returns_none_when_missing() {
        let alpha = Instance::new("Alpha", "/tmp/alpha");

        assert_eq!(
            resolve_instance_id_for_session("aoe_missing", &[alpha]),
            None
        );
    }

    #[test]
    fn test_ordered_profile_session_names_matches_flattened_group_order() {
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
    fn test_ordered_profile_session_names_keeps_sessions_in_collapsed_groups() {
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
            vec![
                crate::tmux::Session::generate_name(&visible.id, &visible.title),
                crate::tmux::Session::generate_name(&hidden.id, &hidden.title),
            ]
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

        let sessions = ordered_profile_session_names(&instances, &groups, SortOrder::Newest);

        assert_eq!(
            sessions,
            vec![
                crate::tmux::Session::generate_name(&visible.id, &visible.title),
                crate::tmux::Session::generate_name(&hidden.id, &hidden.title),
            ]
        );
    }

    #[test]
    fn test_ordered_profile_session_names_are_stable_across_group_collapse_state() {
        let now = Utc::now();
        let visible = instance_with_created_at("Visible", "/tmp/v", now);
        let mut hidden_a =
            instance_with_created_at("Hidden A", "/tmp/ha", now + Duration::seconds(1));
        hidden_a.group_path = "work".to_string();
        let mut hidden_b =
            instance_with_created_at("Hidden B", "/tmp/hb", now + Duration::seconds(2));
        hidden_b.group_path = "work".to_string();
        let instances = vec![visible.clone(), hidden_a.clone(), hidden_b.clone()];

        let expanded_groups = vec![Group {
            name: "work".to_string(),
            path: "work".to_string(),
            collapsed: false,
            default_directory: None,
            children: Vec::new(),
        }];
        let collapsed_groups = vec![Group {
            collapsed: true,
            ..expanded_groups[0].clone()
        }];

        let expanded =
            ordered_profile_session_names(&instances, &expanded_groups, SortOrder::Newest);
        let collapsed =
            ordered_profile_session_names(&instances, &collapsed_groups, SortOrder::Newest);

        assert_eq!(collapsed, expanded);
    }

    #[test]
    fn test_resolve_existing_session_by_index_keeps_stable_slots_for_collapsed_groups() {
        let now = Utc::now();
        let alpha = instance_with_created_at("Alpha", "/tmp/a", now);
        let mut beta = instance_with_created_at("Beta", "/tmp/b", now + Duration::seconds(1));
        beta.group_path = "work".to_string();
        let mut gamma = instance_with_created_at("Gamma", "/tmp/c", now + Duration::seconds(2));
        gamma.group_path = "work".to_string();
        let instances = vec![alpha.clone(), beta.clone(), gamma.clone()];
        let groups = vec![Group {
            name: "work".to_string(),
            path: "work".to_string(),
            collapsed: true,
            default_directory: None,
            children: Vec::new(),
        }];

        let sessions = ordered_profile_session_names(&instances, &groups, SortOrder::AZ);
        let alpha_session = crate::tmux::Session::generate_name(&alpha.id, &alpha.title);
        let beta_session = crate::tmux::Session::generate_name(&beta.id, &beta.title);

        let target =
            resolve_existing_session_by_index(&sessions, 2, |session| session != alpha_session);

        assert_eq!(target, Some(beta_session));
    }

    #[test]
    fn test_session_index_in_order_uses_stable_slot_without_filtering_missing_sessions() {
        let now = Utc::now();
        let alpha = instance_with_created_at("Alpha", "/tmp/a", now);
        let mut beta = instance_with_created_at("Beta", "/tmp/b", now + Duration::seconds(1));
        beta.group_path = "work".to_string();
        let mut gamma = instance_with_created_at("Gamma", "/tmp/c", now + Duration::seconds(2));
        gamma.group_path = "work".to_string();
        let instances = vec![alpha.clone(), beta.clone(), gamma];
        let groups = vec![Group {
            name: "work".to_string(),
            path: "work".to_string(),
            collapsed: true,
            default_directory: None,
            children: Vec::new(),
        }];
        let beta_session = crate::tmux::Session::generate_name(&beta.id, &beta.title);

        assert_eq!(
            session_index_in_order(&instances, &groups, SortOrder::AZ, &beta_session),
            Some(2)
        );
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
        let sessions = ["aoe_a".to_string(), "aoe_b".to_string()];
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
    fn test_back_toggle_run_shell_cmd_contains_back_flag() {
        let cmd = back_toggle_run_shell_cmd("default");
        assert!(cmd.contains("--back"));
        assert!(cmd.contains("--profile"));
    }

    #[test]
    #[serial]
    fn test_track_session_switch_sets_correct_prev_from_title_and_index() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }

        let temp = TempDir::new().unwrap();
        setup_test_home(&temp);

        let mut config = crate::session::config::Config::default();
        config.app_state.sort_order = Some(SortOrder::AZ);
        crate::session::config::save_config(&config).unwrap();

        let alpha = Instance::new("Alpha", "/tmp/alpha");
        let beta = Instance::new("Beta", "/tmp/beta");
        let instances = vec![alpha.clone(), beta.clone()];
        crate::session::Storage::new("default")
            .unwrap()
            .save(&instances)
            .unwrap();

        let alpha_session = crate::tmux::Session::generate_name(&alpha.id, &alpha.title);
        let beta_session = crate::tmux::Session::generate_name(&beta.id, &beta.title);
        let client_name = format!("/tmp/track_client_{}", std::process::id());
        let prev_key = client_context_option_key(AOE_PREV_SESSION_OPTION_PREFIX, &client_name);

        create_tmux_session(&alpha_session);
        create_tmux_session(&beta_session);
        set_tmux_session_option(&alpha_session, AOE_TITLE_OPTION, &alpha.title);
        set_tmux_session_option(&beta_session, AOE_TITLE_OPTION, &beta.title);

        track_session_switch(
            &alpha_session,
            &beta_session,
            Some(&client_name),
            &instances,
            &[],
            SortOrder::AZ,
        );

        assert_eq!(
            get_previous_session_for_client(&client_name),
            Some(alpha_session.clone())
        );
        assert_eq!(
            get_tmux_session_option(&beta_session, AOE_FROM_TITLE_OPTION),
            Some(alpha.title.clone())
        );
        assert_eq!(
            get_tmux_session_option(&beta_session, AOE_INDEX_OPTION),
            Some("2".to_string())
        );

        kill_tmux_session(&alpha_session);
        kill_tmux_session(&beta_session);
        clear_global_option(&prev_key);
    }

    #[test]
    #[serial]
    fn test_clear_from_title_unsets_option() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }

        let temp = TempDir::new().unwrap();
        setup_test_home(&temp);

        let session_name = format!("aoe_clear_from_title_{}", std::process::id());
        create_tmux_session(&session_name);
        set_tmux_session_option(&session_name, AOE_FROM_TITLE_OPTION, "Alpha");

        assert_eq!(
            get_tmux_session_option(&session_name, AOE_FROM_TITLE_OPTION),
            Some("Alpha".to_string())
        );

        clear_from_title(&session_name);

        assert_eq!(
            get_tmux_session_option(&session_name, AOE_FROM_TITLE_OPTION),
            None
        );

        kill_tmux_session(&session_name);
    }

    #[test]
    #[serial]
    fn test_clear_previous_session_for_client_unsets_option() {
        if !tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }

        let temp = TempDir::new().unwrap();
        setup_test_home(&temp);

        let client_name = format!("/tmp/clear_prev_client_{}", std::process::id());
        let option_key = client_context_option_key(AOE_PREV_SESSION_OPTION_PREFIX, &client_name);
        set_global_option(&option_key, "aoe_target");

        assert_eq!(
            get_global_option(&option_key),
            Some("aoe_target".to_string())
        );

        clear_previous_session_for_client(&client_name);

        assert_eq!(get_global_option(&option_key), None);
        clear_global_option(&option_key);
    }

    #[test]
    fn test_switch_to_previous_session_calls_switch_with_previous_session() {
        let switched_to = RefCell::new(None::<String>);

        let target = switch_to_previous_session(
            Some("aoe_current"),
            Some("aoe_previous"),
            |_| true,
            |session| {
                *switched_to.borrow_mut() = Some(session.to_string());
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(target, Some("aoe_previous".to_string()));
        assert_eq!(switched_to.into_inner(), Some("aoe_previous".to_string()));
    }

    #[test]
    fn test_switch_to_previous_session_is_no_op_without_previous_session() {
        let switched = Cell::new(false);

        let target = switch_to_previous_session(
            Some("aoe_current"),
            None,
            |_| true,
            |_| {
                switched.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(target, None);
        assert!(!switched.get());
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
