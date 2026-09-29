use anyhow::{Context, Result};

use super::HomeView;
use crate::session::Status;
use crate::tui::app::PostRestart;
use crate::tui::dialogs::{InfoDialog, SwitchAgentDialog};

impl HomeView {
    pub(super) fn open_switch_agent_dialog(&mut self, post: PostRestart) {
        let Some(inst) = self
            .selected_session
            .as_deref()
            .and_then(|id| self.get_instance(id))
        else {
            return;
        };
        if inst.status == Status::Deleting || inst.restart_in_flight {
            return;
        }
        let refusal = if inst.is_sandboxed() {
            Some("Sandboxed sessions cannot switch agents.".to_string())
        } else if !crate::agents::is_switchable_agent(&inst.tool) {
            Some(format!(
                "'{}' runs {}; only claude and codex sessions can switch agents.",
                inst.title, inst.tool
            ))
        } else {
            None
        };
        if let Some(message) = refusal {
            self.info_dialog = Some(InfoDialog::new("Cannot Switch Agent", &message));
            return;
        }
        let dialog = SwitchAgentDialog::new(&inst.id, &inst.title, &inst.tool);
        self.switch_agent_dialog = Some((dialog, post));
    }

    /// Durably hand the session to `target` before any restart runs: the
    /// restart paths read the agent from the slots and the saved instance, and
    /// the background worker merges back nothing that says which agent it is.
    pub(super) fn commit_agent_switch(&mut self, id: &str, target: &str) -> Result<()> {
        let mut switched = self
            .get_instance(id)
            .cloned()
            .context("session disappeared")?;
        let profile = self.storage.profile().to_string();
        let config = crate::session::profile_config::resolve_config(&profile).unwrap_or_else(|e| {
            tracing::warn!("Failed to load config, using defaults: {}", e);
            crate::session::Config::default()
        });
        let store = crate::db::Store::open_with_schema(&profile)?;
        let session_name = crate::tmux::Session::generate_name(&switched.id, &switched.title);

        // Without a slot, a restart rebuilds the pane from whatever process is
        // still running in it, which is the agent being replaced.
        if store.read_slots_for_instance(id)?.is_empty() {
            if let Some(pane) = crate::tmux::get_agent_pane_id(&session_name) {
                store.record_launched_slot_config_if_absent(
                    id,
                    0,
                    switched.primary_pane_config(),
                    &pane,
                    switched.xats_identity_key.as_deref().unwrap_or_default(),
                    crate::db::now_unix(),
                )?;
            }
        }

        switched.switch_agent(target, &config)?;
        store.switch_instance_agent(id, target)?;
        self.mutate_instance(id, |inst| *inst = switched);
        self.save()
    }
}
