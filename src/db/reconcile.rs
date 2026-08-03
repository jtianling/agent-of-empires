//! Reconciler: snapshot volatile per-pane captures into durable per-slot rows.
//!
//! Driven by the status-poller tick. Per managed session it enumerates the
//! session's tmux panes, resolves each pane's capture via `pane_live`, assigns a
//! deterministic slot (the primary `@aoe_agent_pane` is slot 0; remaining panes
//! by ascending pane index), and upserts an `agent_slot` row. It caps tracking
//! at four slots per session, appends an `adopt` event when a pane is first
//! recorded, and garbage-collects orphan `pane_live` rows whose pane is not in
//! any managed session.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::db::{Store, MAX_SLOT};
use crate::session::Instance;

/// One pane of a managed session, after slot assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignedPane {
    pub pane_id: String,
    pub slot: i64,
}

/// Assign slots to a session's panes, keeping already-tracked panes in place.
///
/// Assignment is sticky: a pane that already owns a slot (per `existing`, the
/// instance's durable `(slot, tmux_pane)` rows) keeps that slot, so a newly
/// appearing pane never evicts an established one. The primary pane
/// (`@aoe_agent_pane`, if live) is pinned to slot 0. A durable slot whose pane
/// is still one of the session's live panes is reserved so it is not
/// overwritten; a durable slot whose pane is gone (the user closed that split,
/// or tmux reused the id elsewhere) is reclaimable, so a new live pane can take
/// it rather than being dropped at the four-slot cap. New live panes fill the
/// remaining free slots in ascending pane-index order; once all `MAX_SLOT + 1`
/// (4) slots are taken, extra panes are dropped.
///
/// `panes` is a list of `(pane_index, pane_id)` sorted by pane index.
pub fn assign_slots(
    panes: &[(u32, String)],
    primary_pane: Option<&str>,
    existing: &[(i64, String)],
) -> Vec<AssignedPane> {
    let mut assigned = Vec::new();
    let mut used_slots: HashSet<i64> = HashSet::new();

    let pane_to_slot: HashMap<&str, i64> = existing.iter().map(|(s, p)| (p.as_str(), *s)).collect();

    // 1. Pin the primary pane to slot 0 if it is one of the session's panes.
    if let Some(primary) = primary_pane {
        if panes.iter().any(|(_, id)| id == primary) {
            assigned.push(AssignedPane {
                pane_id: primary.to_string(),
                slot: 0,
            });
            used_slots.insert(0);
        }
    }

    // 2. Keep live panes in their existing slot (sticky), in pane-index order.
    for (_, pane_id) in panes {
        if Some(pane_id.as_str()) == primary_pane && used_slots.contains(&0) {
            continue;
        }
        if let Some(&slot) = pane_to_slot.get(pane_id.as_str()) {
            if (0..=MAX_SLOT).contains(&slot) && !used_slots.contains(&slot) {
                assigned.push(AssignedPane {
                    pane_id: pane_id.clone(),
                    slot,
                });
                used_slots.insert(slot);
            }
        }
    }

    // 3. Reserve durable slots whose pane is still one of the session's live
    //    panes, so an established record is never overwritten by a different
    //    pane. A durable slot whose pane is gone (the user closed that split, or
    //    tmux reused the id elsewhere) is deliberately NOT reserved: step 4 lets
    //    a new live pane reclaim the freed slot instead of being dropped at the
    //    four-slot cap. This path only runs for a live session (`reconcile_all`
    //    skips sessions with no panes), so a whole-session death after a reboot
    //    never reclaims here -- those slots stay intact for cold-start recovery.
    let live_pane_ids: HashSet<&str> = panes.iter().map(|(_, id)| id.as_str()).collect();
    for (slot, tmux_pane) in existing {
        if (0..=MAX_SLOT).contains(slot) && live_pane_ids.contains(tmux_pane.as_str()) {
            used_slots.insert(*slot);
        }
    }

    // 4. Fill remaining live panes into the lowest free slot, dropping extras.
    let mut next_slot = 0i64;
    for (_, pane_id) in panes {
        if Some(pane_id.as_str()) == primary_pane {
            continue;
        }
        if assigned.iter().any(|a| &a.pane_id == pane_id) {
            continue;
        }
        while used_slots.contains(&next_slot) {
            next_slot += 1;
        }
        if next_slot > MAX_SLOT {
            break;
        }
        assigned.push(AssignedPane {
            pane_id: pane_id.clone(),
            slot: next_slot,
        });
        used_slots.insert(next_slot);
    }

    assigned.sort_by_key(|a| a.slot);
    assigned
}

/// Maximum number of agent panes tracked per session, i.e. the slot range the
/// store accepts. Every entry point that adds a pane enforces the same cap.
pub const MAX_AGENT_PANES: usize = (MAX_SLOT + 1) as usize;

/// An agent pane AoE has just launched, before any capture exists for it.
pub struct LaunchedPane<'a> {
    pub pane_id: &'a str,
    pub config: &'a crate::session::PaneConfig,
    /// The identity key minted for this pane, empty when it gets none.
    pub identity_key: &'a str,
    /// Slot provisioned before the pane process started.
    pub prepared_slot: Option<i64>,
    pub prepared_generation: Option<i64>,
}

/// Record the durable slots of an extra agent pane AoE has just launched and of
/// the primary pane beside it, so the key the launch minted has a home and both
/// panes are restartable before either has been captured.
///
/// Slots come from the same assignment the reconciler uses, so a launch-time row
/// lands in the slot the first capture would have written and is sticky from
/// then on. The extra pane's row is always written: its slot is by construction
/// free or held by a pane that is gone. The primary pane's row is written only
/// when it has none, because an existing one carries a captured conversation
/// that a launch-time row would blank. Its key is persisted in slot 0.
pub fn record_launched_extra_pane(
    store: &Store,
    instance_id: &str,
    session_name: &str,
    primary: &crate::session::PaneConfig,
    primary_identity_key: &str,
    pane: &LaunchedPane<'_>,
) -> Result<()> {
    let panes = list_session_panes(session_name);
    let primary_pane = crate::tmux::get_agent_pane_id(session_name);
    record_launched_extra_pane_among(
        store,
        instance_id,
        &panes,
        primary_pane.as_deref(),
        primary,
        primary_identity_key,
        pane,
    )
}

/// The pane-list half of [`record_launched_extra_pane`], separated from the two
/// tmux queries that supply it so the slot writes can be exercised without a
/// live session.
#[allow(clippy::too_many_arguments)]
fn record_launched_extra_pane_among(
    store: &Store,
    instance_id: &str,
    panes: &[(u32, String)],
    primary_pane: Option<&str>,
    primary: &crate::session::PaneConfig,
    primary_identity_key: &str,
    pane: &LaunchedPane<'_>,
) -> Result<()> {
    let existing_rows = store.read_slots_for_instance(instance_id)?;
    let existing_map: Vec<(i64, String)> = existing_rows
        .iter()
        .map(|s| (s.slot, s.tmux_pane.clone()))
        .collect();
    let assigned = assign_slots(panes, primary_pane, &existing_map);
    let now = crate::db::now_unix();

    // The fan-out restart reads only the slots that exist, so recording the
    // extra pane alone would take the primary pane out of it. The write is
    // insert-if-absent rather than guarded by the read above: an existing row
    // carries a captured conversation, and a capture landing between the read
    // and the write would otherwise be blanked.
    if let Some(primary_pane_id) = primary_pane {
        if assigned
            .iter()
            .any(|a| a.slot == 0 && a.pane_id == primary_pane_id)
        {
            store.record_launched_slot_config_if_absent(
                instance_id,
                0,
                primary,
                primary_pane_id,
                primary_identity_key,
                now,
            )?;
        }
    }

    match (pane.prepared_slot, pane.prepared_generation) {
        (Some(prepared_slot), Some(prepared_generation)) => {
            return store.bind_prepared_slot_pane(
                instance_id,
                prepared_slot,
                prepared_generation,
                pane.identity_key,
                pane.pane_id,
                now,
            );
        }
        (None, None) => {}
        _ => anyhow::bail!("prepared pane is missing its slot reservation token"),
    }
    let Some(assignment) = assigned.iter().find(|a| a.pane_id == pane.pane_id) else {
        anyhow::bail!("pane {} was assigned no slot", pane.pane_id);
    };
    store.record_launched_slot_config(
        instance_id,
        assignment.slot,
        pane.config,
        pane.pane_id,
        pane.identity_key,
        now,
    )
}

/// List a tmux session's pane ids (e.g. `%42`). Used by the delete path to
/// purge `pane_live` rows before the session is killed.
pub fn session_pane_ids(session_name: &str) -> Vec<String> {
    list_session_panes(session_name)
        .into_iter()
        .map(|(_, id)| id)
        .collect()
}

pub fn live_session_pane_ids(session_name: &str) -> Result<Vec<String>> {
    Ok(try_list_session_panes(session_name)?
        .into_iter()
        .map(|(_, id)| id)
        .collect())
}

/// List a tmux session's panes as `(pane_index, pane_id)` sorted by pane index.
/// Returns an empty vec if the session does not exist or tmux is unavailable.
fn list_session_panes(session_name: &str) -> Vec<(u32, String)> {
    try_list_session_panes(session_name).unwrap_or_default()
}

fn try_list_session_panes(session_name: &str) -> Result<Vec<(u32, String)>> {
    let output = crate::tmux::tmux_command()
        .args([
            "list-panes",
            "-t",
            session_name,
            "-F",
            "#{pane_index} #{pane_id}",
        ])
        .output()
        .with_context(|| format!("listing panes for tmux session '{session_name}'"))?;
    if !output.status.success() {
        anyhow::bail!(
            "tmux list-panes failed for session '{}': {}",
            session_name,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut panes: Vec<(u32, String)> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let idx = parts.next()?.parse::<u32>().ok()?;
            let id = parts.next()?.to_string();
            Some((idx, id))
        })
        .collect();
    panes.sort_by_key(|(idx, _)| *idx);
    Ok(panes)
}

/// Reconcile all managed instances of the active profile.
///
/// Best-effort: store/tmux failures for one session do not abort the others.
pub fn reconcile_all(profile: &str, instances: &[Instance]) {
    let store = match Store::open_with_schema(profile) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("reconcile: cannot open store: {}", e);
            return;
        }
    };

    // Stamped before the first pane list is read, and handed to orphan
    // collection at the end. Everything between the two is slow -- a rollout
    // claim per pane shells out, and it runs for every instance -- so a pane
    // created in that span is absent from a list read before it existed. Its
    // capture would then be collected as an orphan while the pane is alive, and
    // for an agent whose captures come from hooks nothing writes it again until
    // the next hook event fires.
    let listed_at = crate::db::now_unix();
    let mut live_panes: HashSet<String> = HashSet::new();

    for inst in instances {
        let session_name = crate::tmux::Session::generate_name(&inst.id, &inst.title);
        let panes = list_session_panes(&session_name);
        if panes.is_empty() {
            continue;
        }
        for (_, id) in &panes {
            live_panes.insert(id.clone());
        }
        let primary = crate::tmux::get_agent_pane_id(&session_name);
        // A Codex pane cannot report itself through hooks, so its capture is
        // derived from Codex's rollout files before the snapshot below. Every
        // pane is offered, in pane-index (creation) order: a Codex pane AoE
        // added beside the primary one is invisible to restart and recovery
        // until it has a capture, and the claim refuses panes not running
        // Codex. Ordering plus the "a claimed thread is never reassigned"
        // rule keeps a later pane from taking an earlier pane's conversation.
        for (_, pane_id) in &panes {
            let is_primary = primary.as_deref() == Some(pane_id.as_str());
            crate::db::codex_rollout::maybe_claim_for_pane(&store, inst, pane_id, is_primary);
        }
        if let Err(e) = reconcile_session(&store, inst, &panes, primary.as_deref()) {
            tracing::debug!("reconcile: session {} failed: {}", inst.id, e);
        }
        capture_coherent_layout(&store, &inst.id, &session_name, &panes);
    }

    gc_orphan_pane_live(&store, &live_panes, listed_at);
}

fn capture_coherent_layout(
    store: &Store,
    instance_id: &str,
    session_name: &str,
    panes: &[(u32, String)],
) {
    let Ok(slots) = store.read_slots_for_instance(instance_id) else {
        return;
    };
    let live: HashSet<&str> = panes.iter().map(|(_, id)| id.as_str()).collect();
    let durable: HashSet<&str> = slots.iter().map(|slot| slot.tmux_pane.as_str()).collect();
    if live != durable || live.is_empty() {
        return;
    }
    let Ok(layout) = crate::tmux::session_window_layout(session_name) else {
        return;
    };
    let Ok(layout_ids) = crate::tmux::layout::pane_ids(&layout) else {
        return;
    };
    let layout_set: HashSet<&str> = layout_ids.iter().map(String::as_str).collect();
    if layout_ids.len() != live.len() || layout_set != live {
        return;
    }
    if let Err(e) = store.upsert_layout_snapshot(instance_id, &layout, crate::db::now_unix()) {
        tracing::debug!(
            "reconcile: persist layout for {} failed: {}",
            instance_id,
            e
        );
    }
}

/// Reconcile a single session's panes into durable slots.
fn reconcile_session(
    store: &Store,
    inst: &Instance,
    panes: &[(u32, String)],
    primary_pane: Option<&str>,
) -> Result<()> {
    let existing_rows = store.read_slots_for_instance(&inst.id)?;
    let existing_map: Vec<(i64, String)> = existing_rows
        .iter()
        .map(|s| (s.slot, s.tmux_pane.clone()))
        .collect();
    // A pane capture carries no identity key, so the upsert below would blank it.
    // Carry the slot's current key forward instead. This read is advisory: the
    // write keeps a key already stored over the one passed here, so a launch that
    // mints one between this read and that write is not undone by it.
    let existing_keys: HashMap<i64, String> = existing_rows
        .iter()
        .map(|s| (s.slot, s.xats_identity_key.clone()))
        .collect();
    let existing_configs: HashMap<i64, crate::session::PaneConfig> = existing_rows
        .iter()
        .map(|slot| (slot.slot, slot.pane_config()))
        .collect();
    // Slots already tracked for this instance: used to detect first-time
    // adoption (a slot that did not exist before) for event logging.
    let existing: HashSet<i64> = existing_rows.iter().map(|s| s.slot).collect();
    // What each slot last captured, so a `capture` event is appended on change
    // rather than on every poll tick. Reconcile runs on the poller cadence, so a
    // per-tick append records that polling happened, not that anything occurred.
    let existing_natives: HashMap<i64, String> = existing_rows
        .iter()
        .map(|s| (s.slot, s.native_session_id.clone()))
        .collect();

    let assigned = assign_slots(panes, primary_pane, &existing_map);

    for pane in &assigned {
        let Some(capture) = store.read_pane_live(&pane.pane_id)? else {
            continue;
        };
        if capture.native_session_id.is_empty() {
            continue;
        }
        let now = crate::db::now_unix();
        let mut pane_config = existing_configs
            .get(&pane.slot)
            .cloned()
            .unwrap_or_else(|| {
                if pane.slot == 0 {
                    inst.primary_pane_config().clone()
                } else {
                    crate::session::PaneConfig::new(
                        capture.agent.clone(),
                        capture.cwd.clone(),
                        false,
                        false,
                    )
                }
            });
        pane_config.tool = capture.agent.clone();
        pane_config.working_dir = capture.cwd.clone();
        let normalized = crate::session::PaneConfig::new(
            pane_config.tool.clone(),
            pane_config.working_dir.clone(),
            pane_config.yolo_mode,
            pane_config.cross_agent_team,
        );
        pane_config.yolo_mode = normalized.yolo_mode;
        pane_config.cross_agent_team = normalized.cross_agent_team;
        if let Err(error) = pane_config.validate() {
            tracing::warn!(
                "reconcile: skipping invalid capture for {} slot {}: {}",
                inst.id,
                pane.slot,
                error
            );
            continue;
        }
        let identity_key = existing_keys
            .get(&pane.slot)
            .map(String::as_str)
            .filter(|key| !key.is_empty())
            .or_else(|| {
                (pane.slot == 0)
                    .then_some(inst.xats_identity_key.as_deref())
                    .flatten()
            })
            .unwrap_or("");
        if let Err(error) = store.upsert_agent_slot_capture_config(
            &inst.id,
            pane.slot,
            &pane_config,
            &capture.native_session_id,
            &pane.pane_id,
            identity_key,
            now,
        ) {
            tracing::warn!(
                "reconcile: failed to persist {} slot {}: {}",
                inst.id,
                pane.slot,
                error
            );
            continue;
        }
        if !existing.contains(&pane.slot) {
            // First time this slot is recorded for the session: adoption.
            store.append_event(
                &inst.id,
                Some(pane.slot),
                "adopt",
                Some(&capture.native_session_id),
                now,
            )?;
        } else if existing_natives.get(&pane.slot).map(String::as_str)
            != Some(capture.native_session_id.as_str())
        {
            store.append_event(
                &inst.id,
                Some(pane.slot),
                "capture",
                Some(&capture.native_session_id),
                now,
            )?;
        }
    }

    Ok(())
}

/// Delete `pane_live` rows whose pane is not in any managed session.
fn gc_orphan_pane_live(store: &Store, live_panes: &HashSet<String>, listed_at: i64) {
    let keys = match store.pane_live_keys_before(listed_at) {
        Ok(k) => k,
        Err(e) => {
            tracing::debug!("reconcile gc: cannot list pane_live: {}", e);
            return;
        }
    };
    for key in keys {
        if !live_panes.contains(&key) {
            if let Err(e) = store.delete_pane_live(&key) {
                tracing::debug!("reconcile gc: delete {} failed: {}", key, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes(ids: &[(u32, &str)]) -> Vec<(u32, String)> {
        ids.iter().map(|(i, s)| (*i, s.to_string())).collect()
    }

    #[test]
    fn primary_pinned_to_slot_zero() {
        let p = panes(&[(0, "%10"), (1, "%11"), (2, "%12")]);
        let assigned = assign_slots(&p, Some("%11"), &[]);
        // Primary %11 -> slot 0; remaining by ascending index.
        assert_eq!(
            assigned,
            vec![
                AssignedPane {
                    pane_id: "%11".into(),
                    slot: 0
                },
                AssignedPane {
                    pane_id: "%10".into(),
                    slot: 1
                },
                AssignedPane {
                    pane_id: "%12".into(),
                    slot: 2
                },
            ]
        );
    }

    #[test]
    fn no_primary_assigns_by_index() {
        let p = panes(&[(0, "%10"), (1, "%11")]);
        let assigned = assign_slots(&p, None, &[]);
        assert_eq!(
            assigned,
            vec![
                AssignedPane {
                    pane_id: "%10".into(),
                    slot: 0
                },
                AssignedPane {
                    pane_id: "%11".into(),
                    slot: 1
                },
            ]
        );
    }

    #[test]
    fn caps_at_four_slots() {
        let p = panes(&[
            (0, "%10"),
            (1, "%11"),
            (2, "%12"),
            (3, "%13"),
            (4, "%14"),
            (5, "%15"),
        ]);
        let assigned = assign_slots(&p, Some("%12"), &[]);
        assert_eq!(assigned.len(), 4, "must cap at four slots");
        let slots: Vec<i64> = assigned.iter().map(|a| a.slot).collect();
        assert_eq!(slots, vec![0, 1, 2, 3]);
        // Primary pinned to slot 0.
        assert_eq!(assigned[0].pane_id, "%12");
    }

    #[test]
    fn primary_not_in_pane_list_is_ignored() {
        let p = panes(&[(0, "%10"), (1, "%11")]);
        let assigned = assign_slots(&p, Some("%99"), &[]);
        // %99 isn't a pane; fall back to index order.
        assert_eq!(
            assigned,
            vec![
                AssignedPane {
                    pane_id: "%10".into(),
                    slot: 0
                },
                AssignedPane {
                    pane_id: "%11".into(),
                    slot: 1
                },
            ]
        );
    }

    #[test]
    fn existing_slots_are_sticky_and_new_pane_dropped_when_full() {
        // Four panes already own slots 0..3; a fifth live pane must be dropped,
        // and the existing four must keep their exact slots.
        let p = panes(&[(0, "%10"), (1, "%11"), (2, "%12"), (3, "%13"), (4, "%14")]);
        let existing = vec![
            (0, "%10".to_string()),
            (1, "%11".to_string()),
            (2, "%12".to_string()),
            (3, "%13".to_string()),
        ];
        let assigned = assign_slots(&p, Some("%10"), &existing);
        assert_eq!(
            assigned,
            vec![
                AssignedPane {
                    pane_id: "%10".into(),
                    slot: 0
                },
                AssignedPane {
                    pane_id: "%11".into(),
                    slot: 1
                },
                AssignedPane {
                    pane_id: "%12".into(),
                    slot: 2
                },
                AssignedPane {
                    pane_id: "%13".into(),
                    slot: 3
                },
            ]
        );
    }

    #[test]
    fn dead_slot_is_reclaimed_by_new_live_pane() {
        // helpers regression: slot 1's pane (%18) died and tmux reused the id in
        // another session; a freshly opened split (%9) must reclaim the freed
        // slot 1 instead of being dropped at the four-slot cap. The three live
        // panes that still own their slots keep them (sticky).
        let p = panes(&[(0, "%6"), (1, "%8"), (2, "%7"), (3, "%9")]);
        let existing = vec![
            (0, "%6".to_string()),
            (1, "%18".to_string()), // dead: not in the live pane list
            (2, "%8".to_string()),
            (3, "%7".to_string()),
        ];
        let assigned = assign_slots(&p, Some("%6"), &existing);
        let mut by_pane: Vec<(&str, i64)> = assigned
            .iter()
            .map(|a| (a.pane_id.as_str(), a.slot))
            .collect();
        by_pane.sort();
        assert_eq!(by_pane, vec![("%6", 0), ("%7", 3), ("%8", 2), ("%9", 1)]);
    }

    #[test]
    fn dead_slot_without_new_pane_leaves_live_panes_untouched() {
        // slot 1 (%18) died but there is no new pane to reclaim it. The live
        // panes keep their existing slots (no needless migration into the gap);
        // the dead slot is simply not reassigned (its DB row is preserved by the
        // caller, which only upserts assigned panes).
        let p = panes(&[(0, "%6"), (1, "%8")]);
        let existing = vec![
            (0, "%6".to_string()),
            (1, "%18".to_string()), // dead
            (2, "%8".to_string()),
        ];
        let assigned = assign_slots(&p, Some("%6"), &existing);
        let mut by_pane: Vec<(&str, i64)> = assigned
            .iter()
            .map(|a| (a.pane_id.as_str(), a.slot))
            .collect();
        by_pane.sort();
        assert_eq!(by_pane, vec![("%6", 0), ("%8", 2)]);
    }

    #[test]
    fn new_low_index_pane_does_not_steal_an_existing_slot() {
        // %11 already owns slot 1. A new pane %99 with a LOWER pane index than
        // %11 must not take slot 1 (sticky); it gets the next free slot.
        let p = panes(&[(0, "%10"), (1, "%99"), (2, "%11")]);
        let existing = vec![(0, "%10".to_string()), (1, "%11".to_string())];
        let assigned = assign_slots(&p, Some("%10"), &existing);
        let mut by_pane: Vec<(&str, i64)> = assigned
            .iter()
            .map(|a| (a.pane_id.as_str(), a.slot))
            .collect();
        by_pane.sort();
        assert_eq!(by_pane, vec![("%10", 0), ("%11", 1), ("%99", 2)]);
    }
}

#[cfg(test)]
mod identity_key_tests {
    use super::*;
    use crate::db::ensure_schema;
    use tempfile::TempDir;

    fn store() -> (TempDir, Store) {
        let tmp = TempDir::new().unwrap();
        let store = Store::open_at(&tmp.path().join("aoe.db")).unwrap();
        ensure_schema(&store.conn).unwrap();
        (tmp, store)
    }

    /// Orphan collection judges captures against a pane list read before the
    /// slow part of a reconcile pass. A pane created in that span is missing
    /// from the list through no fault of its own, and deleting its capture
    /// leaves a live pane untracked until something writes another one, which
    /// for a hook-driven agent may be a long time.
    #[test]
    fn orphan_collection_spares_a_capture_written_after_the_pane_list_was_read() {
        let (_tmp, store) = store();
        let listed_at = 100;

        // Present in the list that was read, and still alive.
        store
            .upsert_pane_live("%1", "claude", "sess-1", "/tmp", 90)
            .unwrap();
        // Absent from that list and older than it: a real orphan.
        store
            .upsert_pane_live("%2", "claude", "sess-2", "/tmp", 90)
            .unwrap();
        // Absent from that list because it did not exist when the list was read.
        store
            .upsert_pane_live("%3", "claude", "sess-3", "/tmp", 110)
            .unwrap();

        let live: HashSet<String> = ["%1".to_string()].into_iter().collect();
        gc_orphan_pane_live(&store, &live, listed_at);

        let mut left = store.all_pane_live_keys().unwrap();
        left.sort();
        assert_eq!(left, vec!["%1".to_string(), "%3".to_string()]);
    }

    /// A capture stamped in the same second as the read is concurrent with it at
    /// the stored resolution, so it is left alone rather than guessed about.
    #[test]
    fn orphan_collection_spares_a_capture_from_the_same_second_as_the_read() {
        let (_tmp, store) = store();

        store
            .upsert_pane_live("%4", "claude", "sess-4", "/tmp", 100)
            .unwrap();

        gc_orphan_pane_live(&store, &HashSet::new(), 100);

        assert_eq!(store.all_pane_live_keys().unwrap(), vec!["%4".to_string()]);
    }

    /// A pane capture carries no identity key, so the reconcile upsert would
    /// blank it. Losing it silently stops the identity from surviving restarts.
    #[test]
    fn reconcile_preserves_an_existing_slot_identity_key() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/recon");

        store
            .upsert_agent_slot(&inst.id, 0, "claude", "sess-0", "/tmp", "%1", "key-0", 1)
            .unwrap();
        store
            .upsert_pane_live("%1", "claude", "sess-0-new", "/tmp", 2)
            .unwrap();

        reconcile_session(&store, &inst, &[(0, "%1".to_string())], Some("%1")).unwrap();

        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        assert_eq!(slots[0].native_session_id, "sess-0-new");
        assert_eq!(
            slots[0].xats_identity_key, "key-0",
            "the capture must not blank the slot's identity key"
        );
    }

    #[test]
    fn invalid_capture_does_not_block_a_valid_sibling() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/recon");
        store
            .upsert_pane_live("%1", "claude", "left", "", 1)
            .unwrap();
        store
            .upsert_pane_live("%2", "codex", "right", "/tmp/right", 1)
            .unwrap();

        reconcile_session(
            &store,
            &inst,
            &[(0, "%1".to_string()), (1, "%2".to_string())],
            Some("%1"),
        )
        .unwrap();

        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].slot, 1);
        assert_eq!(slots[0].agent, "codex");
    }

    #[test]
    fn adopted_secondary_flags_do_not_inherit_from_primary() {
        let (_tmp, store) = store();
        let mut inst = Instance::new("recon", "/tmp/recon");
        inst.set_primary_pane_config(crate::session::PaneConfig::new(
            "claude",
            "/tmp/recon",
            true,
            true,
        ));
        store
            .upsert_pane_live("%1", "claude", "left", "/tmp/recon", 1)
            .unwrap();
        store
            .upsert_pane_live("%2", "shell", "right", "/tmp/right", 1)
            .unwrap();

        reconcile_session(
            &store,
            &inst,
            &[(0, "%1".to_string()), (1, "%2".to_string())],
            Some("%1"),
        )
        .unwrap();

        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        let secondary = slots.iter().find(|slot| slot.slot == 1).unwrap();
        assert_eq!(secondary.agent, "shell");
        assert!(!secondary.yolo_mode);
        assert!(!secondary.cross_agent_team);
    }

    fn event_kinds(store: &Store, instance_id: &str) -> Vec<String> {
        let mut stmt = store
            .conn
            .prepare("SELECT kind FROM events WHERE instance_id = ?1 ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([instance_id], |r| r.get::<_, String>(0))
            .unwrap();
        rows.map(Result::unwrap).collect()
    }

    /// Reconcile runs on the poll cadence. Appending a `capture` event per tick
    /// records that polling happened rather than that anything occurred, and with
    /// no retention it grew one profile's database to several gigabytes.
    #[test]
    fn unchanged_capture_appends_no_event_but_still_refreshes_the_row() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/recon");
        let panes = [(0, "%1".to_string())];

        store
            .upsert_pane_live("%1", "claude", "sess-0", "/tmp", 1)
            .unwrap();
        reconcile_session(&store, &inst, &panes, Some("%1")).unwrap();
        assert_eq!(event_kinds(&store, &inst.id), vec!["adopt"]);

        // Three more ticks with nothing changed.
        for tick in 2..5 {
            store
                .upsert_pane_live("%1", "claude", "sess-0", "/tmp", tick)
                .unwrap();
            reconcile_session(&store, &inst, &panes, Some("%1")).unwrap();
        }

        assert_eq!(
            event_kinds(&store, &inst.id),
            vec!["adopt"],
            "an unchanged capture must not append an event on every tick"
        );
        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        assert_eq!(slots[0].native_session_id, "sess-0");
    }

    #[test]
    fn changed_capture_appends_one_event() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/recon");
        let panes = [(0, "%1".to_string())];

        store
            .upsert_pane_live("%1", "claude", "sess-0", "/tmp", 1)
            .unwrap();
        reconcile_session(&store, &inst, &panes, Some("%1")).unwrap();

        store
            .upsert_pane_live("%1", "claude", "sess-1", "/tmp", 2)
            .unwrap();
        reconcile_session(&store, &inst, &panes, Some("%1")).unwrap();

        assert_eq!(event_kinds(&store, &inst.id), vec!["adopt", "capture"]);
    }

    /// A launch-time record starts with no conversation. The first capture must
    /// complete it in place: a record that were replaced instead would change the
    /// pane's identity at the moment it is adopted.
    #[test]
    fn a_capture_completes_a_launch_time_record_without_replacing_its_key() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/recon");

        store
            .record_launched_slot(&inst.id, 1, "codex", "/tmp/recon", "%2", "launched-key", 1)
            .unwrap();
        store
            .upsert_pane_live("%2", "codex", "first-capture", "/tmp/recon", 2)
            .unwrap();

        reconcile_session(
            &store,
            &inst,
            &[(0, "%1".to_string()), (1, "%2".to_string())],
            Some("%1"),
        )
        .unwrap();

        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        let launched = slots.iter().find(|s| s.slot == 1).expect("launched slot");
        assert_eq!(launched.native_session_id, "first-capture");
        assert_eq!(launched.xats_identity_key, "launched-key");
        assert_eq!(launched.tmux_pane, "%2");
    }

    /// Slot assignment is sticky on the pane id, which the launch-time record
    /// already carries. A record with no capture yet must not drift.
    #[test]
    fn a_launch_time_record_keeps_its_slot_while_its_pane_is_live() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/recon");

        store
            .record_launched_slot(&inst.id, 1, "codex", "/tmp/recon", "%2", "launched-key", 1)
            .unwrap();
        // A third pane appears and is captured first; it must not take slot 1.
        store
            .upsert_pane_live("%3", "claude", "other-capture", "/tmp/recon", 2)
            .unwrap();

        let panes = [
            (0, "%1".to_string()),
            (1, "%2".to_string()),
            (2, "%3".to_string()),
        ];
        reconcile_session(&store, &inst, &panes, Some("%1")).unwrap();

        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        let launched = slots.iter().find(|s| s.slot == 1).expect("launched slot");
        assert_eq!(launched.tmux_pane, "%2");
        assert_eq!(
            launched.native_session_id, "",
            "no capture has arrived for this pane yet"
        );
        assert_eq!(launched.xats_identity_key, "launched-key");
        let other = slots.iter().find(|s| s.slot == 2).expect("other slot");
        assert_eq!(other.tmux_pane, "%3");
    }

    /// The defect this change fixes: a pane launched somewhere other than the
    /// session's directory used to record the session's, which reads correctly
    /// until the first restart puts the pane back in the wrong place.
    #[test]
    fn a_launched_pane_records_its_own_directory_not_the_instance_s() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/session-dir");
        let panes = [(0, "%1".to_string()), (1, "%2".to_string())];
        let primary = crate::session::PaneConfig::new("claude", "/tmp/session-dir", false, false);
        let launched = crate::session::PaneConfig::new("codex", "/tmp/other-dir", false, false);

        record_launched_extra_pane_among(
            &store,
            &inst.id,
            &panes,
            Some("%1"),
            &primary,
            "primary-key",
            &LaunchedPane {
                pane_id: "%2",
                config: &launched,
                identity_key: "launched-key",
                prepared_slot: None,
                prepared_generation: None,
            },
        )
        .unwrap();

        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        let primary = slots.iter().find(|s| s.slot == 0).expect("primary slot");
        let launched = slots.iter().find(|s| s.slot == 1).expect("launched slot");
        assert_eq!(launched.cwd, "/tmp/other-dir");
        assert_eq!(primary.cwd, "/tmp/session-dir");
        assert_eq!(primary.xats_identity_key, "primary-key");
    }

    #[test]
    fn managed_shell_panes_record_their_launch_directory() {
        for shell_cwd in ["/tmp/session-dir", "/tmp/other-dir"] {
            let (_tmp, store) = store();
            let inst = Instance::new("recon", "/tmp/session-dir");
            let panes = [(0, "%1".to_string()), (1, "%2".to_string())];
            let primary =
                crate::session::PaneConfig::new("codex", "/tmp/session-dir", false, false);
            let launched = crate::session::PaneConfig::new("shell", shell_cwd, false, false);

            record_launched_extra_pane_among(
                &store,
                &inst.id,
                &panes,
                Some("%1"),
                &primary,
                "",
                &LaunchedPane {
                    pane_id: "%2",
                    config: &launched,
                    identity_key: "",
                    prepared_slot: None,
                    prepared_generation: None,
                },
            )
            .unwrap();

            let slots = store.read_slots_for_instance(&inst.id).unwrap();
            let shell = slots.iter().find(|slot| slot.slot == 1).unwrap();
            assert_eq!(shell.agent, "shell");
            assert_eq!(shell.cwd, shell_cwd);
            assert!(shell.native_session_id.is_empty());
            assert!(shell.xats_identity_key.is_empty());
        }
    }

    /// The restart fan-out places each pane at the directory its slot recorded,
    /// so the two panes come back where they were launched. Without the
    /// assertion above, the split alone still looks correct.
    #[test]
    fn a_restart_reads_each_pane_s_own_directory_back() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/session-dir");
        let panes = [(0, "%1".to_string()), (1, "%2".to_string())];
        let primary = crate::session::PaneConfig::new("claude", "/tmp/session-dir", false, false);
        let launched = crate::session::PaneConfig::new("codex", "/tmp/other-dir", false, false);

        record_launched_extra_pane_among(
            &store,
            &inst.id,
            &panes,
            Some("%1"),
            &primary,
            "",
            &LaunchedPane {
                pane_id: "%2",
                config: &launched,
                identity_key: "launched-key",
                prepared_slot: None,
                prepared_generation: None,
            },
        )
        .unwrap();

        let mut slots = store.read_slots_for_instance(&inst.id).unwrap();
        slots.sort_by_key(|s| s.slot);
        let relaunch_dirs: Vec<&str> = slots.iter().map(|s| s.cwd.as_str()).collect();
        assert_eq!(relaunch_dirs, ["/tmp/session-dir", "/tmp/other-dir"]);
    }

    /// A pane whose agent moved is corrected by its own report, which is why no
    /// backfill is needed for records written before this change.
    #[test]
    fn a_capture_corrects_a_recorded_directory_and_keeps_the_key() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/session-dir");

        store
            .record_launched_slot(
                &inst.id,
                1,
                "codex",
                "/tmp/other-dir",
                "%2",
                "launched-key",
                1,
            )
            .unwrap();
        store
            .upsert_pane_live("%2", "codex", "first-capture", "/tmp/moved-dir", 2)
            .unwrap();

        reconcile_session(
            &store,
            &inst,
            &[(0, "%1".to_string()), (1, "%2".to_string())],
            Some("%1"),
        )
        .unwrap();

        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        let launched = slots.iter().find(|s| s.slot == 1).expect("launched slot");
        assert_eq!(launched.cwd, "/tmp/moved-dir");
        assert_eq!(launched.xats_identity_key, "launched-key");
    }

    #[test]
    fn reconcile_leaves_a_keyless_slot_empty() {
        let (_tmp, store) = store();
        let inst = Instance::new("recon", "/tmp/recon");

        store
            .upsert_pane_live("%1", "claude", "sess-0", "/tmp", 1)
            .unwrap();

        reconcile_session(&store, &inst, &[(0, "%1".to_string())], Some("%1")).unwrap();

        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        assert_eq!(slots[0].xats_identity_key, "");
    }
}
