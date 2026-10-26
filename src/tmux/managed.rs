use anyhow::{ensure, Context, Result};

pub(crate) fn primary_pane(session: &str) -> Result<String> {
    let output = super::tmux_command()
        .args(primary_pane_args(session))
        .output()
        .context("Reading managed primary pane")?;
    ensure!(
        output.status.success(),
        "Reading managed primary pane failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pane = String::from_utf8(output.stdout).context("Invalid tmux pane response")?;
    let pane = pane.trim();
    ensure!(valid_pane_id(pane), "Invalid managed primary pane ID");
    verify_owner(session, pane)?;
    Ok(pane.to_string())
}

pub(crate) fn verify_owner(session: &str, pane: &str) -> Result<()> {
    ensure!(valid_pane_id(pane), "Invalid managed primary pane ID");
    let output = super::tmux_command()
        .args([
            "display-message",
            "-p",
            "-t",
            pane,
            "#{session_name}|#{pane_id}",
        ])
        .output()
        .context("Verifying managed pane ownership")?;
    ensure!(
        output.status.success(),
        "Verifying managed pane ownership failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let owner = String::from_utf8(output.stdout).context("Invalid tmux ownership response")?;
    validate_owner(session, pane, &owner)
}

fn primary_pane_args(session: &str) -> [String; 5] {
    [
        "show-option".into(),
        "-t".into(),
        format!("={session}:"),
        "-v".into(),
        "@aoe_agent_pane".into(),
    ]
}

fn valid_pane_id(pane: &str) -> bool {
    pane.strip_prefix('%')
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn validate_owner(session: &str, pane: &str, response: &str) -> Result<()> {
    let expected = format!("{session}|{pane}");
    ensure!(
        response.trim_end_matches(['\r', '\n']) == expected,
        "Managed primary pane does not belong to the exact session: expected {expected:?}, received {response:?}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_requires_exact_session_target() {
        assert_eq!(
            primary_pane_args("aoe_worker_12345678"),
            [
                "show-option",
                "-t",
                "=aoe_worker_12345678:",
                "-v",
                "@aoe_agent_pane"
            ]
        );
    }

    #[test]
    fn rejects_other_session_or_pane_and_unsafe_targets() {
        assert!(validate_owner("aoe_worker", "%42", "aoe_worker|%42\n").is_ok());
        for response in ["aoe_worker_extra|%42", "other|%42", "aoe_worker|%43", ""] {
            assert!(validate_owner("aoe_worker", "%42", response).is_err());
        }
        for pane in ["", "%", "%42; kill-server", "aoe_worker", "#{pane_id}"] {
            assert!(!valid_pane_id(pane));
        }
    }
}
