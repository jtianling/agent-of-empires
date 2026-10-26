use crate::xats_pane_lookup::PaneTarget;
use anyhow::{ensure, Context, Result};
use std::time::Duration;

const FORMAT: &str = "#{socket_path}|#{pid}|#{start_time}|#{pane_id}|#{session_name}|#{@aoe_agent_pane}|#{@aoe_managed_pane_id}|#{pane_index}|#{pane_left}|#{pane_top}";

#[derive(Clone)]
pub(crate) struct Pane {
    pub target: PaneTarget,
    pub session: String,
    pub primary: bool,
    pub marker: Option<String>,
    pub index: u32,
    pub left: u32,
    pub top: u32,
}

pub(crate) async fn observe(profile: &str) -> Result<Vec<Pane>> {
    let mut command = tokio::process::Command::from(super::profile_command(profile));
    command
        .args(["list-panes", "-a", "-F", FORMAT])
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .context("Timed out inspecting tmux panes")?
        .context("Inspecting tmux panes")?;
    let detail = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        if detail.contains("no server running on")
            || (detail.contains("error connecting to")
                && detail.contains("No such file or directory"))
        {
            return Ok(Vec::new());
        }
        anyhow::bail!("Cannot inspect tmux panes: {detail}");
    }
    parse(&String::from_utf8(output.stdout).context("Invalid tmux inventory encoding")?)
}

fn parse(output: &str) -> Result<Vec<Pane>> {
    let mut panes = Vec::new();
    for line in output.lines() {
        let fields: Vec<_> = line.split('|').collect();
        ensure!(fields.len() == 10, "Invalid tmux inventory field count");
        let target = PaneTarget {
            socket_path: fields[0].into(),
            server_pid: fields[1].parse()?,
            server_started_at: fields[2].parse()?,
            pane_id: fields[3].into(),
        };
        ensure!(
            std::path::Path::new(&target.socket_path).is_absolute()
                && target.server_pid > 1
                && target.server_started_at > 0,
            "Invalid tmux server identity"
        );
        ensure!(
            valid_pane(&target.pane_id) && !fields[4].is_empty(),
            "Invalid tmux pane identity"
        );
        let marker = (!fields[6].is_empty()).then(|| fields[6].to_string());
        if let Some(marker) = &marker {
            crate::session::managed_pane::validate_pane_id(marker)?;
        }
        ensure!(
            !panes
                .iter()
                .any(|pane: &Pane| pane.target.pane_id == target.pane_id),
            "Duplicate tmux pane ID"
        );
        panes.push(Pane {
            primary: fields[5] == target.pane_id,
            target,
            session: fields[4].into(),
            marker,
            index: fields[7].parse()?,
            left: fields[8].parse()?,
            top: fields[9].parse()?,
        });
    }
    Ok(panes)
}

fn valid_pane(pane: &str) -> bool {
    pane.strip_prefix('%')
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_identity_and_primary_without_title_inference() {
        let panes = parse("/tmp/socket|123|1700000000|%4|session|%4||0|0|0\n").unwrap();
        assert_eq!(panes[0].target.server_pid, 123);
        assert!(panes[0].primary);
        assert_eq!(panes[0].marker, None);
    }

    #[test]
    fn malformed_or_duplicate_inventory_is_not_absence() {
        assert!(parse("/tmp/socket|not-a-pid|0|%4|session|%4||0|0|0").is_err());
        let row = "/tmp/socket|123|1700000000|%4|session|%4||0|0|0\n";
        assert!(parse(&format!("{row}{row}")).is_err());
        assert!(parse("/tmp/socket|123|1700000000|%4|session|%4|invalid|0|0|0").is_err());
    }
}
