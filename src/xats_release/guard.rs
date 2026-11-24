use anyhow::{ensure, Result};
use serde_json::{json, Value};

use crate::db::AgentSlot;
use crate::tmux::inventory::Pane;
use crate::xats_pane_lookup::PaneTarget;

#[derive(Clone, Default)]
pub(crate) struct ReleaseGuard(Option<Snapshot>);

#[derive(Clone)]
struct Snapshot {
    profile: String,
    instance: String,
    session: String,
    panes: Vec<PhysicalPane>,
    slots: Value,
}

#[derive(Clone, PartialEq, Eq)]
struct PhysicalPane {
    target: PaneTarget,
    root_pid: u32,
    dead: bool,
    tty: String,
}

impl ReleaseGuard {
    pub(super) fn new(
        profile: &str,
        instance: &str,
        session: &str,
        panes: &[Pane],
        slots: &[AgentSlot],
    ) -> Self {
        Self(Some(Snapshot {
            profile: profile.into(),
            instance: instance.into(),
            session: session.into(),
            panes: physical(panes),
            slots: slot_fence(slots),
        }))
    }

    pub(crate) fn verify(&self) -> Result<()> {
        if self.0.is_none() {
            return Ok(());
        }
        let guard = self.clone();
        crate::xats_control::block_on_control("xats-release-guard", move || {
            Box::pin(async move { guard.verify_async().await })
        })
    }

    pub(super) async fn verify_async(&self) -> Result<()> {
        let Some(expected) = &self.0 else {
            return Ok(());
        };
        let current =
            crate::tmux::inventory::observe_session(&expected.profile, &expected.session).await?;
        ensure!(
            physical(&current) == expected.panes,
            "Session panes changed during xats release; deletion stopped"
        );
        let slots = crate::db::Store::open_with_schema(&expected.profile)?
            .read_slots_for_instance_with_diagnostics(&expected.instance)?;
        ensure!(
            slots.skipped == 0 && slot_fence(&slots.slots) == expected.slots,
            "Pane identity metadata changed during xats release; deletion stopped"
        );
        Ok(())
    }
}

fn physical(panes: &[Pane]) -> Vec<PhysicalPane> {
    let mut result = panes
        .iter()
        .map(|pane| PhysicalPane {
            target: pane.target.clone(),
            root_pid: pane.root_pid,
            dead: pane.dead,
            tty: pane.tty.clone(),
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| a.target.pane_id.cmp(&b.target.pane_id));
    result
}

fn slot_fence(slots: &[AgentSlot]) -> Value {
    json!(slots
        .iter()
        .map(|slot| (
            slot.slot,
            &slot.agent,
            &slot.native_session_id,
            &slot.tmux_pane,
            &slot.xats_identity_key,
            slot.xats_runtime_generation
        ))
        .collect::<Vec<_>>())
}
