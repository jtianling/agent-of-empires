use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

use super::{Instance, PaneConfig};

mod transition;
pub(crate) use transition::{effective_slot_record, recover_pending_slot};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedSlotTransition {
    pub from_generation: i64,
    pub from_tmux_pane: Option<String>,
    pub resume: bool,
    pub rebind: bool,
    pub fence: Option<crate::managed_xats::Fence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedPaneTarget {
    pub session_id: String,
    pub pane_id: String,
    pub slot: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedPaneReplacement {
    pub pane_id: String,
    pub old_tmux_pane: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedPane {
    pub id: String,
    pub owner: Option<String>,
    pub slot: Option<i64>,
    pub tmux_pane: Option<String>,
    pub config: PaneConfig,
    pub identity_key: String,
    pub xats: Option<crate::managed_xats::ManagedXats>,
    pub native_session_id: Option<String>,
    #[serde(default)]
    pub slot_generation: i64,
    #[serde(default)]
    pub removing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_slot_transition: Option<ManagedSlotTransition>,
}

impl std::fmt::Debug for ManagedPane {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedPane")
            .field("id", &self.id)
            .field("owner", &self.owner)
            .field("slot", &self.slot)
            .field("tmux_pane", &self.tmux_pane)
            .finish()
    }
}

impl ManagedPane {
    pub fn new(
        owner: String,
        config: PaneConfig,
        xats: Option<crate::managed_xats::ManagedXats>,
    ) -> Result<Self> {
        config.validate()?;
        if let Some(identity) = &xats {
            identity.validate()?;
        }
        Ok(Self {
            id: uuid::Uuid::new_v4().simple().to_string()[..16].to_string(),
            owner: Some(owner),
            slot: None,
            tmux_pane: None,
            config,
            identity_key: uuid::Uuid::new_v4().to_string(),
            xats,
            native_session_id: None,
            slot_generation: 0,
            removing: false,
            pending_slot_transition: None,
        })
    }

    pub fn validate(&self) -> Result<()> {
        validate_pane_id(&self.id)?;
        self.config.validate()?;
        ensure!(
            self.slot
                .map_or(true, |slot| (1..=crate::db::MAX_SLOT).contains(&slot)),
            "Invalid managed pane slot"
        );
        ensure!(
            (0..=crate::db::MAX_XATS_RUNTIME_GENERATION).contains(&self.slot_generation),
            "Invalid managed pane generation"
        );
        uuid::Uuid::parse_str(&self.identity_key)?;
        if let Some(owner) = &self.owner {
            uuid::Uuid::parse_str(owner)?;
        }
        if let Some(pane) = &self.tmux_pane {
            crate::tmux::managed_pane::validate_tmux_pane(pane)?;
        }
        if let Some(identity) = &self.xats {
            identity.validate()?;
        }
        self.validate_slot_transition()?;
        Ok(())
    }

    pub fn project(&self, parent: &Instance) -> Result<Instance> {
        self.validate()?;
        let mut instance = Instance::new(&parent.title, &self.config.working_dir);
        instance.id = self.id.clone();
        instance.managed_owner = self.owner.clone();
        instance.managed_xats = self.xats.clone();
        instance.xats_identity_key = Some(self.identity_key.clone());
        instance.agent_session_id = self.native_session_id.clone();
        instance.primary_pane = self.config.clone();
        instance.tool = self.config.tool.clone();
        instance.cross_agent_team = self.config.cross_agent_team;
        instance.yolo_mode = self.config.yolo_mode;
        instance.cross_agent_team_channel = parent.cross_agent_team_channel.clone();
        instance.source_profile = parent.source_profile.clone();
        if let Some(slot) = self.slot {
            instance.managed_pane_target = Some(ManagedPaneTarget {
                session_id: parent.id.clone(),
                pane_id: self.id.clone(),
                slot,
            });
        }
        Ok(instance)
    }

    pub fn capture_runtime(&mut self, runtime: &Instance) -> Result<()> {
        ensure!(
            runtime.id == self.id,
            "Managed runtime belongs to another pane"
        );
        self.xats = runtime.managed_xats.clone();
        self.native_session_id = runtime.agent_session_id.clone();
        Ok(())
    }

    pub fn promote_into(&self, parent: &mut Instance) -> Result<()> {
        self.validate()?;
        parent.managed_primary_id = Some(self.id.clone());
        parent.managed_primary_marked = true;
        parent.managed_xats = self.xats.clone();
        parent.xats_identity_key = Some(self.identity_key.clone());
        parent.agent_session_id = self.native_session_id.clone();
        parent.resume_token = None;
        parent.fork_pending = None;
        parent.tool = self.config.tool.clone();
        parent.project_path = self.config.working_dir.clone();
        parent.primary_pane = self.config.clone();
        parent.yolo_mode = self.config.yolo_mode;
        parent.cross_agent_team = self.config.cross_agent_team;
        parent.command = if self.config.tool == "shell" {
            super::environment::user_shell()
        } else {
            String::new()
        };
        parent.extra_args = String::new();
        parent.worktree_info = self.config.worktree_info().cloned();
        parent.workspace_info = self.config.workspace_info().cloned();
        Ok(())
    }
}

pub fn validate_pane_id(id: &str) -> Result<()> {
    ensure!(
        id.len() == 16 && id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Use the complete 16-character managed pane ID"
    );
    Ok(())
}

pub fn has_managed_state(parent: &Instance) -> bool {
    parent.managed_owner.is_some()
        || parent.managed_xats.is_some()
        || parent.managed_primary_id.is_some()
        || parent.managed_primary_marked
        || parent.managed_primary_replacement.is_some()
        || !parent.managed_panes.is_empty()
}

pub(crate) fn verify_binding(
    profile: &str,
    parent: &Instance,
    pane: &ManagedPane,
) -> Result<String> {
    use anyhow::Context;
    pane.validate()?;
    ensure!(
        pane.pending_slot_transition.is_none(),
        "Managed pane slot transition is incomplete; retry restart or remove"
    );
    let slot = pane.slot.context("Managed pane has no slot")?;
    let target = pane
        .tmux_pane
        .as_deref()
        .context("Managed pane has not launched")?;
    let store = crate::db::Store::open_with_schema(profile)?;
    let row = store
        .read_slots_for_instance(&parent.id)?
        .into_iter()
        .find(|row| row.slot == slot)
        .context("Managed pane slot disappeared")?;
    ensure!(
        row.tmux_pane == target
            && row.xats_identity_key == pane.identity_key
            && row.xats_runtime_generation == pane.slot_generation,
        "Managed pane slot binding is stale"
    );
    let session = crate::tmux::Session::generate_name(&parent.id, &parent.title);
    crate::tmux::managed_pane::verify(&session, target, &pane.id)?;
    Ok(target.to_string())
}

pub(crate) fn require_owned_panes(profile: &str, parent: &Instance, owner: &str) -> Result<()> {
    ensure!(
        parent
            .managed_panes
            .iter()
            .all(|pane| pane.owner.as_deref() == Some(owner)),
        "Session contains a pane owned by another manager or a human"
    );
    for pane in &parent.managed_panes {
        verify_stored_location(parent, pane)?;
    }
    let effective = parent
        .managed_panes
        .iter()
        .map(|pane| effective_slot_record(profile, parent, pane))
        .collect::<Result<Vec<_>>>()?;
    let session = crate::tmux::Session::generate_name(&parent.id, &parent.title);
    let store = crate::db::Store::open_with_schema(profile)?;
    let rows = store.read_slots_for_instance(&parent.id)?;
    if let Some(primary) = rows
        .iter()
        .find(|row| row.slot == 0 && !row.tmux_pane.is_empty())
    {
        require_stored_location(
            &session,
            parent.primary_pane_id(),
            &primary.tmux_pane,
            !parent.managed_primary_marked,
        )?;
    }
    for row in rows.iter().filter(|row| row.slot != 0) {
        ensure!(
            effective.iter().any(|pane| pane.slot == Some(row.slot)
                && pane.identity_key == row.xats_identity_key
                && pane.slot_generation == row.xats_runtime_generation),
            "Session contains an unmanaged stored pane in slot {}",
            row.slot
        );
    }
    if !crate::tmux::Session::new(&parent.id, &parent.title)?.exists() {
        return Ok(());
    }
    let primary = crate::tmux::managed::primary_pane(&session)?;
    ensure!(
        rows.iter()
            .any(|row| row.slot == 0 && row.tmux_pane == primary),
        "Primary pane slot is stale"
    );
    for live in crate::tmux::managed_pane::live_panes(&session)? {
        if live.tmux_pane == primary {
            ensure!(
                !parent.managed_primary_marked
                    || live.marker.as_deref() == Some(parent.primary_pane_id()),
                "Primary pane identity was replaced"
            );
            continue;
        }
        let pane = effective.iter().find(|pane| {
            pane.tmux_pane.as_deref() == Some(&live.tmux_pane)
                && live.marker.as_deref() == Some(&pane.id)
        });
        ensure!(
            pane.is_some(),
            "Session contains protected pane {}",
            live.tmux_pane
        );
        verify_binding(profile, parent, pane.unwrap())?;
    }
    Ok(())
}

pub(crate) fn verify_stored_location(parent: &Instance, pane: &ManagedPane) -> Result<()> {
    pane.validate()?;
    if let Some(target) = &pane.tmux_pane {
        let session = crate::tmux::Session::generate_name(&parent.id, &parent.title);
        require_stored_location(&session, &pane.id, target, false)?;
    }
    Ok(())
}

fn require_stored_location(session: &str, id: &str, target: &str, legacy: bool) -> Result<()> {
    if let Some(location) = crate::tmux::managed_pane::locate(target)? {
        validate_stored_location(
            session,
            id,
            &location.session,
            location.marker.as_deref(),
            legacy,
        )?;
    }
    Ok(())
}

fn validate_stored_location(
    session: &str,
    id: &str,
    actual: &str,
    marker: Option<&str>,
    legacy: bool,
) -> Result<()> {
    ensure!(
        actual == session && (marker == Some(id) || (legacy && marker.is_none())),
        "A managed pane moved to another session or its identity was replaced"
    );
    Ok(())
}

pub fn prepare_children_release(parent: &mut Instance) -> Result<()> {
    for index in 0..parent.managed_panes.len() {
        let mut runtime = parent.managed_panes[index].project(parent)?;
        let result = crate::managed_xats::prepare_release(&mut runtime);
        parent.managed_panes[index].capture_runtime(&runtime)?;
        result?;
    }
    Ok(())
}

pub fn release_children(parent: &Instance) -> Result<()> {
    for pane in &parent.managed_panes {
        let runtime = pane.project(parent)?;
        crate::managed_xats::release(&runtime)?;
        crate::managed_codex::cleanup(&runtime)?;
        crate::managed_runtimes::cleanup(&runtime)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_operations_reject_moved_or_replaced_stored_panes() {
        assert!(validate_stored_location("owned", "pane", "owned", Some("pane"), false).is_ok());
        assert!(validate_stored_location("owned", "pane", "foreign", Some("pane"), false).is_err());
        assert!(validate_stored_location("owned", "pane", "owned", Some("other"), false).is_err());
        assert!(validate_stored_location("owned", "pane", "foreign", None, true).is_err());
        assert!(validate_stored_location("owned", "pane", "owned", None, true).is_ok());
    }

    #[test]
    fn foreign_owner_is_rejected_before_pending_transition_resolution() {
        let owner = uuid::Uuid::new_v4().to_string();
        let mut parent = Instance::new("parent", "/tmp");
        parent.managed_owner = Some(owner.clone());
        let mut foreign = pane(&uuid::Uuid::new_v4().to_string());
        foreign.pending_slot_transition = Some(ManagedSlotTransition {
            from_generation: 1,
            from_tmux_pane: Some("%1".into()),
            resume: true,
            rebind: false,
            fence: None,
        });
        parent.managed_panes.push(foreign);
        let error = require_owned_panes("unused-pending-test", &parent, &owner).unwrap_err();
        assert!(format!("{error:#}").contains("another manager or a human"));
    }

    fn pane(owner: &str) -> ManagedPane {
        ManagedPane::new(
            owner.into(),
            PaneConfig::new("claude", "/tmp", false, true),
            Some(crate::managed_xats::ManagedXats::new("child".into(), "team".into()).unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn projection_preserves_parent_identity_and_isolates_child_credentials() {
        let owner = uuid::Uuid::new_v4().to_string();
        let mut parent = Instance::new("parent", "/tmp");
        parent.managed_owner = Some(owner.clone());
        parent.xats_identity_key = Some(uuid::Uuid::new_v4().to_string());
        parent.source_profile = "test".into();
        parent.command = "unsafe override".into();
        parent.extra_args = "--resume other".into();
        let mut child = pane(&owner);
        child.slot = Some(1);
        child.slot_generation = 2;
        child.tmux_pane = Some("%4".into());
        let projected = child.project(&parent).unwrap();
        assert_eq!(projected.id, child.id);
        assert_ne!(projected.xats_identity_key, parent.xats_identity_key);
        assert!(projected.command.is_empty() && projected.extra_args.is_empty());
        assert_eq!(projected.managed_pane_target.unwrap().session_id, parent.id);
        assert_eq!(projected.source_profile, "test");
        assert!(!format!("{child:?}").contains(&child.identity_key));
    }

    #[test]
    fn promotion_keeps_survivor_logical_identity_and_parent_owner() {
        let owner = uuid::Uuid::new_v4().to_string();
        let mut parent = Instance::new("parent", "/tmp");
        parent.managed_owner = Some(owner.clone());
        let session_id = parent.id.clone();
        let old_primary = parent.primary_pane_id().to_string();
        let child = pane(&owner);
        child.promote_into(&mut parent).unwrap();
        assert_eq!(parent.id, session_id);
        assert_ne!(parent.primary_pane_id(), old_primary);
        assert_eq!(parent.primary_pane_id(), child.id);
        assert_eq!(parent.managed_owner.as_deref(), Some(owner.as_str()));
        assert_eq!(parent.managed_xats, child.xats);
        assert_eq!(
            parent.xats_identity_key.as_deref(),
            Some(child.identity_key.as_str())
        );
    }

    #[test]
    #[serial_test::serial]
    fn promoted_shell_restarts_with_the_actual_user_shell() {
        let owner = uuid::Uuid::new_v4().to_string();
        let mut parent = Instance::new("parent", "/tmp");
        parent.command = "old-agent --unsafe-override".into();
        parent.extra_args = "--old-agent-argument".into();
        let child =
            ManagedPane::new(owner, PaneConfig::new("shell", "/tmp", false, false), None).unwrap();
        let previous = std::env::var_os("SHELL");
        std::env::set_var("SHELL", "/bin/bash");
        let promoted = child.promote_into(&mut parent);
        let command = parent.build_agent_command(None);
        match previous {
            Some(value) => std::env::set_var("SHELL", value),
            None => std::env::remove_var("SHELL"),
        }
        promoted.unwrap();
        assert_eq!(parent.get_tool_command(), "/bin/bash");
        assert!(parent.extra_args.is_empty());
        let command = command.unwrap();
        let words = shell_words::split(&command).unwrap();
        let script = words.last().unwrap();
        assert!(script.contains("/bin/bash"), "{script}");
        assert!(!script.contains("env shell"), "{script}");
        assert!(!script.contains("old-agent"), "{script}");
    }

    #[test]
    fn stored_records_reject_untrusted_slot_and_identifier_values() {
        let mut child = pane(&uuid::Uuid::new_v4().to_string());
        child.slot = Some(0);
        assert!(child.validate().is_err());
        child.slot = Some(1);
        child.tmux_pane = Some("%4;kill-server".into());
        assert!(child.validate().is_err());
        for id in ["", "%1", "left", "1234", "0123456789abcdef00"] {
            assert!(validate_pane_id(id).is_err());
        }
    }
}
