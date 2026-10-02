use std::collections::HashMap;

use anyhow::{Context, Result};

use super::HomeView;
use crate::session::{Instance, Status};
use crate::tui::app::{Action, PostRestart};
use crate::tui::dialogs::{InfoDialog, SwitchAgentDialog, SwitchChoice, SwitchPane};
use crate::xats_identity::KeyHolder;

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
        let panes = if inst.is_sandboxed() {
            Vec::new()
        } else {
            self.switch_panes(inst)
        };
        let refusal = if inst.is_sandboxed() {
            Some("Sandboxed sessions cannot switch agents.".to_string())
        } else if panes.is_empty() {
            Some(format!(
                "'{}' runs {}; only claude and codex panes can switch agents.",
                inst.title, inst.tool
            ))
        } else if panes.len() > 1 && !session_running(inst) {
            // Bringing a stopped session back restarts every pane, which would
            // not leave the other panes as they are.
            Some(format!(
                "'{}' is not running; start it before switching one of its panes.",
                inst.title
            ))
        } else {
            None
        };
        if let Some(message) = refusal {
            self.info_dialog = Some(InfoDialog::new("Cannot Switch Agent", &message));
            return;
        }
        let dialog = SwitchAgentDialog::new(&inst.id, &inst.title, panes);
        self.switch_agent_dialog = Some((dialog, post));
    }

    /// The panes of `inst` a switch can target, in on-screen order.
    fn switch_panes(&self, inst: &Instance) -> Vec<SwitchPane> {
        let slots = crate::db::Store::open_with_schema(self.storage.profile())
            .and_then(|store| store.read_slots_for_instance(&inst.id));
        let primary = || SwitchSeat {
            slot: 0,
            agent: inst.tool.clone(),
            key: inst.xats_identity_key.clone().unwrap_or_default(),
            pane: String::new(),
            xats: inst.cross_agent_team,
        };
        let (seats, read_error) = match slots {
            Ok(slots) if !slots.is_empty() => (
                slots
                    .into_iter()
                    .map(|slot| SwitchSeat {
                        slot: slot.slot,
                        agent: slot.agent,
                        key: slot.xats_identity_key,
                        pane: slot.tmux_pane,
                        // Rows recorded before panes carried their own config
                        // say nothing; the session's setting covers them.
                        xats: slot.cross_agent_team || inst.cross_agent_team,
                    })
                    .collect(),
                None,
            ),
            Ok(_) => (vec![primary()], None),
            Err(e) => (
                vec![primary()],
                Some(format!("reading panes failed: {e:#}")),
            ),
        };
        let mut seats: Vec<SwitchSeat> = seats
            .into_iter()
            .filter(|seat| crate::agents::is_switchable_agent(&seat.agent))
            .collect();
        seats.sort_by_key(|seat| seat.slot);

        let positions = if seats.len() > 1 {
            crate::tmux::pane_positions(&crate::tmux::Session::generate_name(&inst.id, &inst.title))
        } else {
            HashMap::new()
        };
        let corners: Vec<Option<(u32, u32)>> = seats
            .iter()
            .map(|seat| positions.get(&seat.pane).copied())
            .collect();
        let labels = pane_labels(&seats, &corners);
        let identities = seat_identities(&seats, read_error);

        let mut panes: Vec<(Option<(u32, u32)>, SwitchPane)> = seats
            .into_iter()
            .zip(labels)
            .zip(identities)
            .zip(corners)
            .map(|(((seat, label), identity), corner)| {
                (
                    corner,
                    SwitchPane {
                        slot: seat.slot,
                        label,
                        agent: seat.agent,
                        identity,
                    },
                )
            })
            .collect();
        if panes.iter().all(|(corner, _)| corner.is_some()) {
            panes.sort_by_key(|(corner, _)| *corner);
        }
        panes.into_iter().map(|(_, pane)| pane).collect()
    }

    /// Commit every chosen pane, then fresh-restart just those panes, or the
    /// whole session the usual way when it is not running.
    ///
    /// A failed commit stops short of any restart: a restart reports its own
    /// outcome over the session's error, which would hide the failure.
    pub(super) fn commit_agent_switches(
        &mut self,
        id: &str,
        choices: &[SwitchChoice],
        post: PostRestart,
    ) -> Option<Action> {
        let mut committed = Vec::new();
        for choice in choices {
            if let Err(e) = self.commit_agent_switch(id, choice.slot, &choice.target) {
                tracing::error!(
                    "Failed to switch pane {} of '{}' to {}: {}",
                    choice.slot + 1,
                    id,
                    choice.target,
                    e
                );
                let done = if committed.is_empty() {
                    String::new()
                } else {
                    let panes: Vec<String> = committed
                        .iter()
                        .map(|slot: &i64| (slot + 1).to_string())
                        .collect();
                    format!(
                        "; pane {} switched but not restarted, press c",
                        panes.join(", ")
                    )
                };
                self.set_instance_error(
                    id,
                    Some(format!(
                        "Agent switch of pane {} failed: {e:#}{done}",
                        choice.slot + 1
                    )),
                );
                return None;
            }
            committed.push(choice.slot);
        }
        let running = self.get_instance(id).is_some_and(session_running);
        if running {
            return Some(Action::RespawnSlots(id.to_string(), committed, post));
        }
        self.restart_action_for(id, crate::session::RestartMode::Fresh, post)
    }

    /// Durably hand pane `slot` to `target` before any restart runs: the
    /// restart paths read the agent from the slots and the saved instance, and
    /// the background worker merges back nothing that says which agent it is.
    /// Only the primary pane (slot 0) is mirrored on the instance itself.
    pub(super) fn commit_agent_switch(&mut self, id: &str, slot: i64, target: &str) -> Result<()> {
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
        let mut tracked = !store.read_slots_for_instance(id)?.is_empty();
        if !tracked {
            if let Some(pane) = crate::tmux::get_agent_pane_id(&session_name) {
                store.record_launched_slot_config_if_absent(
                    id,
                    0,
                    switched.primary_pane_config(),
                    &pane,
                    switched.xats_identity_key.as_deref().unwrap_or_default(),
                    crate::db::now_unix(),
                )?;
                tracked = true;
            }
        }

        // A session that never ran has no pane to hand over yet; its next
        // start reads the agent from the instance alone.
        if tracked || slot != 0 {
            store.switch_slot_agent(id, slot, target)?;
        }
        if slot != 0 {
            return Ok(());
        }
        switched.switch_agent(target, &config)?;
        self.mutate_instance(id, |inst| *inst = switched);
        self.save()
    }
}

/// Whether the tmux session of `inst` exists, so a single pane can be
/// respawned in place.
fn session_running(inst: &Instance) -> bool {
    inst.tmux_session()
        .map(|session| session.exists())
        .unwrap_or(false)
}

struct SwitchSeat {
    slot: i64,
    agent: String,
    key: String,
    pane: String,
    xats: bool,
}

/// Name each pane the way the user sees it: two panes side by side (or
/// stacked) by where they sit, anything else by number.
fn pane_labels(seats: &[SwitchSeat], corners: &[Option<(u32, u32)>]) -> Vec<String> {
    let numbered = || {
        seats
            .iter()
            .map(|seat| format!("Pane {}", seat.slot + 1))
            .collect()
    };
    let [Some((left_a, top_a)), Some((left_b, top_b))] = corners else {
        return numbered();
    };
    let first_is = |a: u32, b: u32, before: &str, after: &str| {
        if a < b {
            vec![before.to_string(), after.to_string()]
        } else {
            vec![after.to_string(), before.to_string()]
        }
    };
    if left_a != left_b {
        first_is(*left_a, *left_b, "Left", "Right")
    } else if top_a != top_b {
        first_is(*top_a, *top_b, "Top", "Bottom")
    } else {
        numbered()
    }
}

/// What xats identity each pane will come back as.
///
/// The new agent recovers its identity only through the pane's key, and
/// nothing guarantees xats has that key on the identity the pane runs today,
/// so the answer is asked of xats rather than assumed.
fn seat_identities(seats: &[SwitchSeat], read_error: Option<String>) -> Vec<Option<KeyHolder>> {
    if let Some(error) = read_error {
        return seats
            .iter()
            .map(|seat| seat.xats.then(|| KeyHolder::Unavailable(error.clone())))
            .collect();
    }
    let keys: Vec<String> = seats
        .iter()
        .filter(|seat| seat.xats && !seat.key.is_empty())
        .map(|seat| seat.key.clone())
        .collect();
    let mut holders = crate::xats_identity::lookup_holders(keys).into_iter();
    seats
        .iter()
        .map(|seat| match (seat.xats, seat.key.is_empty()) {
            (false, _) => None,
            (true, true) => Some(KeyHolder::NotFound),
            (true, false) => holders.next(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(slot: i64) -> SwitchSeat {
        SwitchSeat {
            slot,
            agent: "claude".to_string(),
            key: String::new(),
            pane: String::new(),
            xats: false,
        }
    }

    #[test]
    fn two_panes_are_named_by_where_they_sit() {
        let seats = [seat(0), seat(1)];
        assert_eq!(
            pane_labels(&seats, &[Some((0, 0)), Some((81, 0))]),
            vec!["Left", "Right"]
        );
        assert_eq!(
            pane_labels(&seats, &[Some((81, 0)), Some((0, 0))]),
            vec!["Right", "Left"]
        );
        assert_eq!(
            pane_labels(&seats, &[Some((0, 20)), Some((0, 0))]),
            vec!["Bottom", "Top"]
        );
    }

    #[test]
    fn panes_without_a_clear_position_are_numbered() {
        let two = [seat(0), seat(1)];
        assert_eq!(
            pane_labels(&two, &[Some((0, 0)), None]),
            vec!["Pane 1", "Pane 2"]
        );
        let three = [seat(0), seat(1), seat(3)];
        assert_eq!(
            pane_labels(&three, &[Some((0, 0)), Some((81, 0)), Some((81, 20))]),
            vec!["Pane 1", "Pane 2", "Pane 4"]
        );
    }
}
