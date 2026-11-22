use anyhow::{ensure, Context, Result};
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub(super) struct CaptureRequests(BTreeSet<String>);

impl CaptureRequests {
    pub(super) fn insert(&mut self, id: &str) {
        if id
            .strip_prefix('%')
            .is_some_and(|n| n.parse::<u32>().is_ok_and(|value| value.to_string() == n))
        {
            self.0.insert(id.to_owned());
        } else {
            tracing::warn!("Invalid monitor capture pane ID: {id:?}");
        }
    }

    pub(super) fn capture(self) -> HashMap<String, String> {
        self.capture_with(capture_pane)
    }

    fn capture_with(self, mut read: impl FnMut(&str) -> Result<String>) -> HashMap<String, String> {
        let mut screens = HashMap::new();
        for id in self.0 {
            match read(&id) {
                Ok(screen) => {
                    screens.insert(id, screen);
                }
                Err(error) => tracing::warn!("Monitor capture for {id} failed: {error:#}"),
            }
        }
        screens
    }
}

fn capture_pane(id: &str) -> Result<String> {
    let output = crate::tmux::control::output(&["capture-pane", "-p", "-S", "-50", "-t", id])
        .context("Reading tmux pane")?;
    ensure!(
        output.status.success(),
        "tmux capture-pane returned {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    ensure!(!output.stdout.is_empty(), "Missing capture payload");
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_pane_does_not_discard_other_captures() {
        let mut requests = CaptureRequests::default();
        for id in ["%2", "%1", "%2", "%01", "session:0"] {
            requests.insert(id);
        }
        let mut calls = Vec::new();
        let content = "%end 1 2 3\n\n";
        let screens = requests.capture_with(|id| {
            calls.push(id.to_owned());
            ensure!(id != "%1", "Pane disappeared");
            Ok(content.to_owned())
        });
        assert_eq!(calls, ["%1", "%2"]);
        assert_eq!(screens, HashMap::from([("%2".into(), content.into())]));
    }
}
