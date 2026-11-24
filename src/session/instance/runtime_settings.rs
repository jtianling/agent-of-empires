use anyhow::{Context, Result};

use super::Instance;
use crate::db::{runtime_settings::RuntimeSettings, AgentSlot, Store};

impl Instance {
    fn settings_owner(&self) -> (&str, Option<i64>) {
        match &self.managed_pane_target {
            Some(target) => (&target.session_id, Some(target.slot)),
            None => (&self.id, None),
        }
    }

    pub(crate) fn pane_runtime_settings(
        &self,
        agent: &str,
        primary: bool,
        identity: Option<&str>,
        native: Option<&str>,
    ) -> Result<Option<RuntimeSettings>> {
        if agent == "antigravity" && primary {
            let args = shell_words::split(&self.extra_args)
                .context("Invalid persisted Antigravity arguments")?;
            crate::agents::antigravity::validate_args(&args)?;
        }
        if agent != "codex" {
            return Ok(None);
        }
        let store = Store::open_with_schema(&self.managed_profile())?;
        let (owner, projected_slot) = self.settings_owner();
        let slot = store
            .read_slots_for_instance(owner)?
            .into_iter()
            .find(|slot| {
                slot.agent == agent
                    && match projected_slot {
                        Some(index) => slot.slot == index,
                        None if primary => slot.slot == 0,
                        None => {
                            identity
                                .filter(|id| !id.is_empty())
                                .is_some_and(|id| slot.xats_identity_key == id)
                                || native
                                    .filter(|id| !id.is_empty())
                                    .is_some_and(|id| slot.native_session_id == id)
                        }
                    }
            });
        let Some(slot) = slot else { return Ok(None) };
        if let Some(settings) = store.runtime_settings(&slot)? {
            return Ok(Some(settings));
        }
        if slot.native_session_id.is_empty() {
            return Ok(None);
        }
        let settings = crate::codex_permissions::settings::capture_persisted(
            &slot.native_session_id,
            std::path::Path::new(&slot.cwd),
        )?
        .context("Cannot restore Codex pane settings; no snapshot is available")?;
        store.save_runtime_settings(&slot, &settings)?;
        Ok(Some(settings))
    }

    pub(crate) fn capture_runtime_settings(&self) -> Result<()> {
        self.capture_selected_settings(false)
    }

    pub(super) fn capture_primary_runtime_settings(&self) -> Result<()> {
        self.capture_selected_settings(true)
    }

    fn capture_selected_settings(&self, primary_only: bool) -> Result<()> {
        if self.is_sandboxed() {
            return Ok(());
        }
        let store = Store::open_with_schema(&self.managed_profile())?;
        let (owner, projected_slot) = self.settings_owner();
        for slot in store.read_slots_for_instance(owner)? {
            if projected_slot.is_some_and(|index| slot.slot != index) {
                continue;
            }
            if primary_only && projected_slot.is_none() && slot.slot != 0 {
                continue;
            }
            self.capture_slot_runtime_settings(&store, &slot, true)?;
        }
        Ok(())
    }

    pub(super) fn capture_slot_runtime_settings(
        &self,
        store: &Store,
        slot: &AgentSlot,
        live_source: bool,
    ) -> Result<()> {
        if self.is_sandboxed() || slot.agent != "codex" || slot.native_session_id.is_empty() {
            return Ok(());
        }
        let captured = if !live_source {
            Self::offline_slot_settings(store, slot)
        } else if slot.cross_agent_team {
            self.capture_remote_settings(store, slot)
        } else {
            crate::codex_permissions::settings::capture_persisted(
                &slot.native_session_id,
                std::path::Path::new(&slot.cwd),
            )
            .and_then(|settings| {
                if settings.is_none() && !self.tmux_session()?.exists() {
                    store.runtime_settings(slot)
                } else {
                    Ok(settings)
                }
            })
        };
        let settings = captured.map_err(|error| {
            tracing::error!(
                instance_id = slot.instance_id,
                slot = slot.slot,
                "Cannot capture Codex pane settings: {error:#}"
            );
            error.context("Pane settings capture failed; restart canceled")
        })?;
        if let Some(settings) = settings {
            store.save_runtime_settings(slot, &settings)?;
        } else {
            anyhow::bail!("Cannot preserve Codex pane settings; no fresh snapshot is available");
        }
        Ok(())
    }

    pub(super) fn offline_slot_settings(
        store: &Store,
        slot: &AgentSlot,
    ) -> Result<Option<RuntimeSettings>> {
        match crate::codex_permissions::settings::capture_persisted(
            &slot.native_session_id,
            std::path::Path::new(&slot.cwd),
        )? {
            Some(settings) => Ok(Some(settings)),
            None => store.runtime_settings(slot),
        }
    }

    fn capture_remote_settings(
        &self,
        store: &Store,
        slot: &AgentSlot,
    ) -> Result<Option<RuntimeSettings>> {
        let projected = self
            .managed_panes
            .iter()
            .find(|pane| pane.slot == Some(slot.slot))
            .map(|pane| pane.project(self))
            .transpose()?;
        let runtime = projected.as_ref().unwrap_or(self);
        let live = if runtime.managed_xats.is_some() {
            crate::managed_codex::has_live_server(runtime)?
        } else {
            !slot.tmux_pane.is_empty()
                && crate::db::codex_capture::current_launch_id(&slot.tmux_pane)?.is_some()
        };
        if !live {
            tracing::debug!(
                slot = slot.slot,
                "Codex runtime is offline; retaining pane settings"
            );
            return Self::offline_slot_settings(store, slot);
        }
        let endpoint = match crate::managed_xats::runtime(runtime) {
            Some(runtime) => runtime
                .get("ws_url")
                .and_then(|url| url.as_str())
                .context("Managed Codex runtime has no endpoint")?
                .to_string(),
            None => {
                super::codex_app_server_endpoint()
                    .map_err(anyhow::Error::msg)?
                    .url
            }
        };
        crate::codex_permissions::settings::capture(endpoint, slot.native_session_id.clone())
    }
}
