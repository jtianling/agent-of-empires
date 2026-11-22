use std::path::Path;

use anyhow::{ensure, Context, Result};

use crate::{process, tmux};

fn query(profile: &str, args: &[&str]) -> Result<Vec<u8>> {
    let output = tmux::profile_command(profile)
        .args(["-N", "-u"])
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "tmux legacy owner handoff failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim_end()
    );
    Ok(output.stdout)
}

fn safe_expression(mut text: &str) -> bool {
    while let Some((_, tail)) = text.split_once('#') {
        let Some((field, rest)) = tail.strip_prefix('{').and_then(|s| s.split_once('}')) else {
            return false;
        };
        if field.is_empty()
            || !field
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return false;
        }
        text = rest;
    }
    true
}

fn monitor_args(args: &[String], exe: &Path, profile: &str) -> bool {
    matches!(args, [program, cmd, subcommand, flag, name]
        if Path::new(program) == exe && cmd == "tmux" && subcommand == "monitor-notifications"
            && flag == "--profile" && name == profile)
}

fn witness(pid: u32, exe: &Path, profile: &str) -> Result<Option<String>> {
    let Some(start) = process::start_identity(pid)? else {
        return Ok(None);
    };
    if process::get_process_comm(pid).as_deref() != exe.file_name().and_then(|s| s.to_str()) {
        return Ok(None);
    }
    let args = process::arguments(pid);
    if process::start_identity(pid)?.as_ref() != Some(&start) {
        return Ok(None);
    }
    Ok(monitor_args(&args?, exe, profile).then_some(start))
}

fn owner<'a>(row: &'a [u8], literal: &str) -> Result<Option<(&'a str, u32)>> {
    if row.is_empty() {
        return Ok(None);
    }
    let (key, value) = std::str::from_utf8(row)?
        .strip_suffix('\n')
        .and_then(|s| s.rsplit_once(' '))
        .context("Malformed R5 owner option")?;
    let pid = value.parse::<u32>()?;
    ensure!(
        (2..=i32::MAX as u32).contains(&pid) && value == pid.to_string(),
        "Invalid R5 owner PID"
    );
    ensure!(
        key.starts_with(super::NOTIFICATION_MONITOR_PID_OPTION_PREFIX)
            && !key.contains(['#', '{', '}', '\\'])
            && !key.chars().any(char::is_control),
        "Cannot safely reference returned R5 owner key {key:?}"
    );
    Ok((key != literal).then_some((key, pid)))
}

pub(super) fn retire(profile: &str, literal: &str, new_pid: &str) {
    if !profile.contains('#') {
        return;
    }
    if !safe_expression(profile) || profile.ends_with(';') {
        tracing::debug!("Keeping any R5 owner for {profile:?}: unsupported legacy expression");
        return;
    }
    if let Err(error) = retire_verified(profile, literal, new_pid) {
        tracing::warn!("Keeping R5 owner for {profile:?}: {error:#}");
    }
}

fn retire_verified(profile: &str, literal: &str, new_pid: &str) -> Result<()> {
    let row = query(profile, &["show-options", "-gq", "--", literal])?;
    let Some((key, old)) = owner(&row, literal)? else {
        return Ok(());
    };
    if old.to_string() == new_pid {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let Some(start) = witness(old, &exe, profile)? else {
        tracing::debug!("Keeping R5 owner {key:?}: PID {old} is not the expected live monitor");
        return Ok(());
    };
    let new_key = tmux::control::literal_format(literal);
    let old_key = tmux::control::literal_format(key);
    let mut checks = vec!["show-options", "-gqv", new_key.as_ref()];
    for scope in ["-sqv", "-gwqv", "-gqv"] {
        checks.extend([";", "show-options", scope, old_key.as_ref()]);
    }
    if query(profile, &checks)? != format!("{new_pid}\n{old}\n").as_bytes()
        || witness(old, &exe, profile)? != Some(start)
    {
        tracing::debug!("Keeping R5 owner {key:?}: option scope, owner or process changed");
        return Ok(());
    }
    let branch = key.replace(',', "#{a:44}");
    // A pane-local override must not authorize retiring a changed global owner.
    let mismatches = format!("#{{S:#{{W:#{{P:#{{?#{{==:#{{{key}}},{old}}},,x}}}}}}}}");
    let guard = format!("#{{?#{{&&:#{{==:#{{{key}}},{old}}},#{{==:{mismatches},}}}},{branch},}}");
    query(profile, &["set-option", "-gqu", "--", &guard])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_direct_field_lookups_are_evaluated() {
        assert!(safe_expression("p,#{pid}.中文 #{session_name}"));
        for name in [
            "p#(ignored)",
            "#{E:key}",
            "#{?a,b,c}",
            "##{pid}",
            "#{#{pid}}",
            "#{pid",
            "#{}",
            "p#S",
            "#{pid}#(ignored)",
        ] {
            assert!(!safe_expression(name), "{name}");
        }
    }

    #[test]
    fn missing_exact_result_does_not_select_another_key() {
        let literal = "@aoe_notification_monitor_pid_p,#{pid}";
        assert_eq!(owner(b"", literal).unwrap(), None);
        assert_eq!(
            owner(b"@aoe_notification_monitor_pid_p,123 45\n", literal).unwrap(),
            Some(("@aoe_notification_monitor_pid_p,123", 45))
        );
        assert!(owner(b"@aoe_notification_monitor_pid_p#{pid} 45\n", literal).is_err());
    }

    #[test]
    fn requires_exact_program_subcommand_and_profile() {
        let exe = Path::new("/bin/aoe");
        let mut args = [
            "/bin/aoe",
            "tmux",
            "monitor-notifications",
            "--profile",
            "p#{pid}",
        ]
        .map(String::from)
        .to_vec();
        assert!(monitor_args(&args, exe, "p#{pid}"));
        assert!(!monitor_args(&args, exe, "p123"));
        assert!(!monitor_args(&args, Path::new("/other/aoe"), "p#{pid}"));
        args.push("--extra".into());
        assert!(!monitor_args(&args, exe, "p#{pid}"));
    }
}
