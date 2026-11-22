use std::time::Instant;

use anyhow::{ensure, Context, Result};

use super::titles;
use crate::tmux::{
    parse_pane_info_cache_output, parse_session_cache_output, PaneInfoCache, SessionCache,
    PANE_FORMAT, PANE_INFO_CACHE, SESSION_CACHE, SESSION_FORMAT,
};

pub(super) struct Snapshot {
    pub owner: String,
    pub server: (u32, u64),
    pub release_unattached_server: bool,
    pub active: titles::Inventory,
}

fn query(args: &[&str]) -> Result<Vec<u8>> {
    let command = args[0];
    let output = crate::tmux::control::output(args)
        .with_context(|| format!("Reading monitor inventory with tmux {command}"))?;
    ensure!(
        output.status.success(),
        "Monitor inventory tmux {command} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

pub(super) fn read(owner_option: &str) -> Result<Snapshot> {
    let owner = query(&["show-options", "-gqv", owner_option])?;
    let server = query(&[
        "display-message",
        "-p",
        "#{pid} #{start_time} #{exit-unattached}",
    ])?;
    let server = std::str::from_utf8(&server)?;
    let mut fields = server.split_whitespace();
    let pid = fields.next().context("Missing tmux server PID")?;
    let started = fields.next().context("Missing tmux server start time")?;
    let release_unattached_server = match fields.next() {
        Some("1" | "on") => query(&[
            "list-clients",
            "-f",
            "#{?@aoe_control_observer,0,1}",
            "-F",
            "#{client_name}",
        ])?
        .is_empty(),
        Some("0" | "off") => false,
        _ => anyhow::bail!("Invalid tmux exit-unattached setting"),
    };
    let pid: u32 = pid.parse()?;
    ensure!(pid > 1 && pid <= i32::MAX as u32, "Invalid tmux server PID");
    let server = (pid, started.parse()?);
    let sessions = query(&["list-sessions", "-F", SESSION_FORMAT])?;
    let panes = query(&["list-panes", "-a", "-F", PANE_FORMAT])?;
    let active = query(&[
        "list-panes",
        "-a",
        "-f",
        "#{&&:#{window_active},#{pane_active}}",
        "-F",
        titles::ACTIVE_PANE_FORMAT,
    ])?;
    let time = Some(Instant::now());
    *SESSION_CACHE
        .write()
        .map_err(|_| anyhow::anyhow!("Session cache poisoned"))? = SessionCache {
        data: Some(parse_session_cache_output(&sessions)),
        time,
    };
    *PANE_INFO_CACHE
        .write()
        .map_err(|_| anyhow::anyhow!("Pane cache poisoned"))? = PaneInfoCache {
        data: Some(parse_pane_info_cache_output(&panes)),
        time,
    };
    Ok(Snapshot {
        owner: String::from_utf8_lossy(&owner).trim().to_owned(),
        server,
        release_unattached_server,
        active: titles::parse_inventory(&String::from_utf8_lossy(&active)),
    })
}

pub(super) fn notification_differs(name: &str, desired: &str) -> Result<bool> {
    const MAX_VALUE_BYTES: usize = 64 * 1024;
    let len = desired.len();
    ensure!(
        len <= MAX_VALUE_BYTES,
        "Desired notification exceeds {MAX_VALUE_BYTES} bytes"
    );
    // The l: modifier keeps the reference literal inside the comparison.
    let mut format = String::from("#{session_name}\t#{==:#{@aoe_waiting},#{l:");
    for ch in desired.chars() {
        if matches!(ch, '#' | ',' | '}') {
            format.push('#');
        }
        format.push(ch);
    }
    format.push_str("}}.");
    let target = format!("={name}:");
    // list-panes expands the format without strftime processing literal '%' bytes.
    let output = query(&[
        "list-panes",
        "-t",
        &target,
        "-f",
        "#{pane_active}",
        "-F",
        &format,
    ])?;
    notification_result(name, &output)
}

fn notification_result(name: &str, output: &[u8]) -> Result<bool> {
    match output.strip_prefix(name.as_bytes()) {
        Some(b"\t1.\n") => Ok(false),
        Some(b"\t0.\n") => Ok(true),
        _ => anyhow::bail!("Invalid notification comparison response for {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_complete_comparisons_for_the_target_decide_writes() {
        assert!(!notification_result("aoe_a", b"aoe_a\t1.\n").unwrap());
        assert!(notification_result("aoe_a", b"aoe_a\t0.\n").unwrap());
        for reply in [
            b"aoe_a\t0".as_slice(),
            b"aoe_a\t.\n",
            b"aoe_ab\t0.\n",
            b"aoe_a\t0.\nextra",
            b"aoe_a\t1\n",
        ] {
            assert!(notification_result("aoe_a", reply).is_err());
        }
    }
}
