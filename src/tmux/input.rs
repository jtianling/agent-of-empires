use std::io::Write;
use std::process::{Output, Stdio};

use anyhow::{bail, ensure, Context, Result};

use super::{send_keys_to_pane_target, tmux_command};

pub fn submit_text_to_pane_target(pane: &str, text: &str) -> Result<()> {
    let buffer = format!("aoe-send-{}", uuid::Uuid::new_v4());
    let result = load_buffer(&buffer, text).and_then(|()| {
        run_tmux(
            &["paste-buffer", "-p", "-r", "-d", "-b", &buffer, "-t", pane],
            "Pasting input to tmux pane",
        )
    });
    if let Err(error) = result {
        return match run_tmux(
            &["delete-buffer", "-b", &buffer],
            "Removing tmux send buffer",
        ) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(anyhow::anyhow!("{error:#}; {cleanup:#}")),
        };
    }
    send_keys_to_pane_target(pane, &["Enter"])
        .context("Input was pasted, but sending the submit key failed")
}

fn load_buffer(buffer: &str, text: &str) -> Result<()> {
    let mut child = tmux_command()
        .args(["load-buffer", "-b", buffer, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("Starting tmux send buffer loader")?;
    let written = child
        .stdin
        .take()
        .context("Tmux send buffer loader has no stdin")
        .and_then(|mut input| {
            input
                .write_all(text.as_bytes())
                .context("Writing input to tmux send buffer")
        });
    let loaded = child
        .wait_with_output()
        .context("Waiting for tmux send buffer loader")
        .and_then(|output| check_output(output, "Loading tmux send buffer"));
    match (written, loaded) {
        (Err(write), Err(load)) => bail!("{write:#}; {load:#}"),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn run_tmux(args: &[&str], action: &str) -> Result<()> {
    let output = tmux_command()
        .args(args)
        .output()
        .with_context(|| format!("{action}: starting tmux"))?;
    check_output(output, action)
}

fn check_output(output: Output, action: &str) -> Result<()> {
    ensure!(
        output.status.success(),
        "{action} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}
