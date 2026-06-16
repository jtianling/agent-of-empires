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
use std::process::Command;

use anyhow::Result;

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
/// (`@aoe_agent_pane`, if live) is pinned to slot 0. Slots already recorded in
/// `existing` are reserved even when their pane is no longer live, so durable
/// records are preserved. New live panes fill the remaining free slots in
/// ascending pane-index order; once all `MAX_SLOT + 1` (4) slots are taken,
/// extra panes are dropped.
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

    // 3. Reserve durable slots not re-kept above (their pane died or moved) so
    //    a new pane never overwrites an existing record.
    for (slot, _) in existing {
        if (0..=MAX_SLOT).contains(slot) {
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

/// List a tmux session's pane ids (e.g. `%42`). Used by the delete path to
/// purge `pane_live` rows before the session is killed.
pub fn session_pane_ids(session_name: &str) -> Vec<String> {
    list_session_panes(session_name)
        .into_iter()
        .map(|(_, id)| id)
        .collect()
}

/// List a tmux session's panes as `(pane_index, pane_id)` sorted by pane index.
/// Returns an empty vec if the session does not exist or tmux is unavailable.
fn list_session_panes(session_name: &str) -> Vec<(u32, String)> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            session_name,
            "-F",
            "#{pane_index} #{pane_id}",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
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
    panes
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
        if let Err(e) = reconcile_session(&store, inst, &panes, primary.as_deref()) {
            tracing::debug!("reconcile: session {} failed: {}", inst.id, e);
        }
    }

    gc_orphan_pane_live(&store, &live_panes);
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
    // Slots already tracked for this instance: used to detect first-time
    // adoption (a slot that did not exist before) for event logging.
    let existing: HashSet<i64> = existing_rows.iter().map(|s| s.slot).collect();

    let assigned = assign_slots(panes, primary_pane, &existing_map);

    for pane in &assigned {
        let Some(capture) = store.read_pane_live(&pane.pane_id)? else {
            continue;
        };
        if capture.native_session_id.is_empty() {
            continue;
        }
        let now = crate::db::now_unix();
        store.upsert_agent_slot(
            &inst.id,
            pane.slot,
            &capture.agent,
            &capture.native_session_id,
            &capture.cwd,
            &pane.pane_id,
            now,
        )?;
        if !existing.contains(&pane.slot) {
            // First time this slot is recorded for the session: adoption.
            store.append_event(
                &inst.id,
                Some(pane.slot),
                "adopt",
                Some(&capture.native_session_id),
                now,
            )?;
        } else {
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
fn gc_orphan_pane_live(store: &Store, live_panes: &HashSet<String>) {
    let keys = match store.all_pane_live_keys() {
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
