use anyhow::{ensure, Context, Result};

const MARKER: &str = "@aoe_managed_pane_id";
const READY: &str = "@aoe_managed_launch_ready";

pub(crate) struct PaneLocation {
    pub session: String,
    pub marker: Option<String>,
}

pub(crate) fn locate(pane: &str) -> Result<Option<PaneLocation>> {
    validate_tmux_pane(pane)?;
    let output = super::tmux_command()
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_id}|#{@aoe_managed_pane_id}|#{session_name}",
        ])
        .output()
        .context("Locating exact pane before removal")?;
    let detail = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        if detail.contains("no server running on")
            || (detail.contains("error connecting to")
                && detail.contains("No such file or directory"))
        {
            return Ok(None);
        }
        anyhow::bail!("Locating pane failed: {detail}");
    }
    let text = String::from_utf8(output.stdout).context("Invalid pane location encoding")?;
    for line in text.lines() {
        let parts: Vec<_> = line.splitn(3, '|').collect();
        if parts.first().copied() != Some(pane) {
            continue;
        }
        ensure!(
            parts.len() == 3 && !parts[2].is_empty(),
            "Invalid pane location schema"
        );
        let marker = if parts[1].is_empty() {
            None
        } else {
            crate::session::managed_pane::validate_pane_id(parts[1])?;
            Some(parts[1].to_string())
        };
        return Ok(Some(PaneLocation {
            session: parts[2].to_string(),
            marker,
        }));
    }
    Ok(None)
}

#[derive(Debug, Clone)]
pub(crate) struct LivePane {
    pub tmux_pane: String,
    pub index: u32,
    pub left: u32,
    pub top: u32,
    pub marker: Option<String>,
}

pub(crate) fn live_panes(session: &str) -> Result<Vec<LivePane>> {
    let output = super::tmux_command()
        .args([
            "list-panes",
            "-s",
            "-t",
            &format!("={session}:"),
            "-F",
            "#{pane_id}|#{pane_index}|#{pane_left}|#{pane_top}|#{@aoe_managed_pane_id}",
        ])
        .output()
        .context("Listing exact session panes")?;
    ensure!(
        output.status.success(),
        "Listing panes failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).context("Invalid pane listing encoding")?;
    text.lines().map(parse_pane).collect()
}

fn parse_pane(line: &str) -> Result<LivePane> {
    let fields: Vec<_> = line.split('|').collect();
    ensure!(fields.len() == 5, "Invalid pane listing schema");
    validate_tmux_pane(fields[0])?;
    let marker = if fields[4].is_empty() {
        None
    } else {
        crate::session::managed_pane::validate_pane_id(fields[4])?;
        Some(fields[4].to_string())
    };
    Ok(LivePane {
        tmux_pane: fields[0].to_string(),
        index: fields[1].parse()?,
        left: fields[2].parse()?,
        top: fields[3].parse()?,
        marker,
    })
}

pub(crate) fn validate_tmux_pane(pane: &str) -> Result<()> {
    ensure!(
        pane.strip_prefix('%')
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())),
        "Invalid exact tmux pane ID"
    );
    Ok(())
}

pub(crate) fn verify(session: &str, pane: &str, id: &str) -> Result<()> {
    validate_tmux_pane(pane)?;
    crate::session::managed_pane::validate_pane_id(id)?;
    ensure!(
        live_panes(session)?
            .iter()
            .any(|entry| entry.tmux_pane == pane && entry.marker.as_deref() == Some(id)),
        "Managed pane binding is stale or belongs to another session"
    );
    Ok(())
}

pub(crate) fn mark(session: &str, pane: &str, id: &str) -> Result<()> {
    validate_tmux_pane(pane)?;
    crate::session::managed_pane::validate_pane_id(id)?;
    super::managed::verify_owner(session, pane)?;
    let output = super::tmux_command()
        .args(["set-option", "-p", "-t", pane, MARKER, id])
        .output()
        .context("Marking managed pane identity")?;
    ensure!(
        output.status.success(),
        "Marking pane failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    verify(session, pane, id)
}

pub(crate) fn mark_ready(session: &str, pane: &str, id: &str, generation: i64) -> Result<()> {
    verify(session, pane, id)?;
    ensure!(generation >= 0, "Invalid ready generation");
    let value = format!("{id}:{generation}");
    let output = super::tmux_command()
        .args(["set-option", "-p", "-t", pane, READY, &value])
        .output()
        .context("Recording managed launch readiness")?;
    ensure!(
        output.status.success(),
        "Recording pane readiness failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

pub(crate) fn clear_ready(session: &str, pane: &str, id: &str) -> Result<()> {
    verify(session, pane, id)?;
    let output = super::tmux_command()
        .args(["set-option", "-p", "-u", "-t", pane, READY])
        .output()
        .context("Clearing managed launch readiness")?;
    ensure!(
        output.status.success(),
        "Clearing pane readiness failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

pub(crate) fn is_ready(session: &str, pane: &str, id: &str, generation: i64) -> Result<bool> {
    verify(session, pane, id)?;
    let output = super::tmux_command()
        .args(["show-option", "-p", "-q", "-v", "-t", pane, READY])
        .output()
        .context("Reading managed launch readiness")?;
    ensure!(
        output.status.success(),
        "Reading pane readiness failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim() == format!("{id}:{generation}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_listing_rejects_unsafe_or_malformed_identity() {
        assert!(parse_pane("%3|1|80|0|0123456789abcdef").is_ok());
        assert!(parse_pane("%4|2|120|0|").unwrap().marker.is_none());
        for value in ["left|1|0|0|", "%3|1|0|0|short", "%3|bad|0|0|", "%3|1|0|0||"] {
            assert!(parse_pane(value).is_err());
        }
    }
}
