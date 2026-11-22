use anyhow::{ensure, Context, Result};
use std::collections::HashMap;
use std::time::Instant;

use crate::session::{Instance, Status};
use crate::tmux::{status_bar, status_detection, SESSION_PREFIX};

const LEGACY_OWNER: &str = "@aoe_codex_title_monitor_pid";
pub(super) const ACTIVE_PANE_FORMAT: &str = concat!(
    "#{session_name}\t#{session_id}\t#{pane_id}\t#{window_activity}\t",
    "#{?@aoe_codex_title_monitor_pid,1,0}\t",
    "#{@aoe_agent_pane}\t#{pane_index}\t",
    "#{pane_current_command}\t#{pane_title}"
);

pub(super) struct ActivePane {
    session_id: String,
    id: String,
    activity: i64,
    legacy_owner: bool,
    agent_pane: Option<String>,
    index: u32,
    command: String,
    title: String,
}

#[derive(Clone, Debug)]
pub(super) struct Sample {
    pane_id: String,
    activity: i64,
    sampled_at: Instant,
    status: Status,
}

pub(super) type Inventory = HashMap<String, Option<ActivePane>>;

pub(super) fn parse_inventory(output: &str) -> Inventory {
    output
        .lines()
        .filter(|row| row.starts_with(SESSION_PREFIX))
        .map(|row| {
            let (name, _) = row.split_once('\t').unwrap_or((row, ""));
            let pane = match ActivePane::parse(row) {
                Ok(pane) => Some(pane),
                Err(error) => {
                    tracing::warn!("Skipping title for {name}: {error:#}");
                    None
                }
            };
            (name.to_owned(), pane)
        })
        .collect()
}

impl ActivePane {
    fn parse(row: &str) -> Result<Self> {
        let fields: Vec<_> = row.splitn(9, '\t').collect();
        let [name, session, id, activity, owner, agent_pane, index, command, title] =
            fields.as_slice()
        else {
            anyhow::bail!("Incomplete active pane metadata");
        };
        ensure!(
            session
                .strip_prefix('$')
                .and_then(|s| s.parse::<u32>().ok())
                .is_some()
                && id
                    .strip_prefix('%')
                    .and_then(|s| s.parse::<u32>().ok())
                    .is_some()
                && matches!(*owner, "0" | "1"),
            "Invalid tmux identifiers or legacy owner flag for {name}"
        );
        Ok(Self {
            session_id: session.to_string(),
            id: id.to_string(),
            activity: activity.parse().context("Invalid window activity")?,
            legacy_owner: *owner == "1",
            agent_pane: Some(agent_pane.trim())
                .filter(|value| !value.is_empty() && !value.contains('#'))
                .map(str::to_owned),
            index: index.parse().context("Invalid pane index")?,
            command: command.to_string(),
            title: title.to_string(),
        })
    }

    fn manages_title(&self, instance: &Instance) -> bool {
        if instance.tool != "codex" {
            return false;
        }
        match status_detection::detect_agent_type_from_command(&self.command) {
            Some("shell") | None => {}
            Some(tool) => return tool == "codex",
        }
        if let Some(pane) = instance
            .managed_panes
            .iter()
            .find(|pane| pane.tmux_pane.as_deref() == Some(&self.id))
        {
            return pane.config.tool == "codex";
        }
        self.agent_pane
            .as_ref()
            .map_or(self.index == 0, |id| id == &self.id)
    }

    fn needs_capture(&self, previous: &Sample, now: Instant) -> bool {
        self.id != previous.pane_id
            || self.activity != previous.activity
            || now.duration_since(previous.sampled_at) >= super::FULL_CHECK_INTERVAL
    }

    pub(super) fn fresh_primary_status(
        &self,
        sample: Option<&Sample>,
        now: Instant,
    ) -> Option<Status> {
        sample?.status_for(self.primary_id(), now)
    }

    pub(super) fn primary_id(&self) -> &str {
        self.agent_pane.as_deref().unwrap_or(&self.id)
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn request_title_capture(
        &self,
        instance: &Instance,
        sample: Option<&Sample>,
        now: Instant,
        requests: &mut super::captures::CaptureRequests,
    ) {
        if self.manages_title(instance) && sample.map_or(true, |prev| self.needs_capture(prev, now))
        {
            requests.insert(&self.id);
        }
    }

    pub(super) fn refresh_title(
        &self,
        instance: &Instance,
        sample: &mut Option<Sample>,
        now: Instant,
        screens: &HashMap<String, String>,
    ) -> Result<()> {
        if self.legacy_owner {
            // Older monitors observe the missing owner and exit on their next tick.
            status_bar::unset_session_option(&self.session_id, LEGACY_OWNER)?;
        }
        if !self.manages_title(instance) {
            *sample = None;
            return Ok(());
        }
        if let Some(content) = screens.get(&self.id) {
            *sample = Some(Sample {
                pane_id: self.id.clone(),
                activity: self.activity,
                sampled_at: now,
                status: status_detection::detect_codex_status(content),
            });
        } else if sample
            .as_ref()
            .map_or(true, |prev| self.needs_capture(prev, now))
        {
            return Ok(());
        }
        if let Some(sample) = sample {
            let desired =
                status_bar::managed_agent_pane_title("codex", &instance.title, sample.status);
            if self.title != desired {
                status_bar::set_pane_title(&self.id, &desired)?;
            }
        }
        Ok(())
    }
}

impl Sample {
    pub(super) fn status_for(&self, id: &str, now: Instant) -> Option<Status> {
        (self.pane_id == id && self.sampled_at == now).then_some(self.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn malformed_title_metadata_preserves_session_membership() {
        let panes = parse_inventory(
            "aoe_bad\t$1\t%2\t?\t1\t%3\t0\tcodex\tx\naoe_good\t$2\t%3\t42\t0\t%3\t0\tcodex\ty",
        );
        assert!(panes["aoe_bad"].is_none());
        assert!(panes["aoe_good"].is_some());
    }

    #[test]
    fn live_title_owner_overrides_recorded_codex_tool() {
        use crate::session::{ManagedPane, PaneConfig};
        let mut instance = Instance::new("test", "/tmp");
        instance.tool = "codex".to_owned();
        let config = PaneConfig::new("codex", "/tmp", false, false);
        let mut tracked = ManagedPane::new("test".into(), config, None).unwrap();
        tracked.tmux_pane = Some("%3".to_owned());
        instance.managed_panes.push(tracked);
        let pane = ActivePane::parse("aoe_a\t$2\t%3\t42\t0\t%1\t1\tclaude\tCustom").unwrap();
        assert!(!pane.manages_title(&instance));
    }

    #[test]
    fn unchanged_active_pane_reuses_capture_until_full_check() {
        let pane = ActivePane::parse("aoe_a\t$2\t%3\t42\t1\t%3\t0\tcodex\tTitle").unwrap();
        let now = Instant::now();
        let sample = Sample {
            pane_id: pane.id.clone(),
            activity: pane.activity,
            sampled_at: now,
            status: Status::Waiting,
        };
        assert_eq!(
            pane.fresh_primary_status(Some(&sample), now),
            Some(Status::Waiting)
        );
        let unbound = ActivePane::parse("aoe_a\t$2\t%3\t42\t0\t\t1\tcodex\tOther").unwrap();
        assert_eq!(
            unbound.fresh_primary_status(Some(&sample), now),
            Some(Status::Waiting)
        );
        assert_eq!(sample.status_for("%4", now), None);
        assert_eq!(sample.status_for("%3", now + Duration::from_secs(1)), None);
        assert!(!pane.needs_capture(&sample, now + Duration::from_secs(1)));
        assert!(pane.needs_capture(&sample, now + super::super::FULL_CHECK_INTERVAL));
        let switched = ActivePane::parse("aoe_a\t$2\t%4\t42\t0\t%3\t1\tcodex\tOther").unwrap();
        assert!(switched.needs_capture(&sample, now));
        let active = ActivePane::parse("aoe_a\t$2\t%3\t43\t0\t%3\t0\tcodex\tTitle").unwrap();
        assert!(active.needs_capture(&sample, now));
    }
}
