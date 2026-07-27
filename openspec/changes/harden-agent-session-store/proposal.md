## Why

A profile's `aoe.db` reached 3.8 GB and then became unreadable, and AoE could no longer start in that profile at all.

Two independent defects produced that.

The reconciler appends a `capture` event on every poll tick for every tracked slot, whether or not anything changed, and nothing ever removes event rows. Deleting a session does not remove its events either. An append-only stream that records heartbeats instead of events grows without bound: one busy profile accumulated several hundred megabytes a day until its database was too large to be useful and, once its tail was truncated, unreadable.

Nothing then contained the damage. Every routine caller of the store logs and moves on when it cannot be opened, so a corrupt database stayed invisible while that profile's capture and reconcile silently did nothing. The first startup path that opened it and propagated the failure turned a dormant problem into a binary that refused to launch, with a bare SQLite message and no way forward.

The store holds derived state: captures, slot assignments, an event log, and a layout snapshot. None of it justifies making AoE unusable.

## What Changes

- Append a `capture` event only when a pane's captured session id actually changes, so the stream records events rather than heartbeats.
- Prune events on schema application: drop rows older than a retention window, and cap the rows kept per instance so a single busy instance cannot dominate the table.
- Remove a session's event rows when its durable records are deleted.
- Detect a corrupt or unreadable database when applying the schema, move it aside with a timestamped name, and recreate an empty one so the profile stays usable.
- Surface the quarantine as a visible warning naming the preserved file, rather than a silent recovery or a bare error.
- Reclaim space after pruning so a database that has already grown shrinks instead of merely freeing pages internally.

## Capabilities

### Modified Capabilities

- `agent-session-store`: the event stream gains retention, session deletion purges events, and schema application quarantines and recreates an unreadable database instead of failing.
- `pane-session-capture`: reconcile records a capture event only when the captured session id changes.

## Impact

- `src/db/mod.rs` for retention, deletion, corruption detection, quarantine, and space reclamation.
- `src/db/reconcile.rs` for change-gated capture events.
- `src/tui/` for surfacing the quarantine warning.
- Unit tests for retention, purge-on-delete, change-gating, and quarantine; e2e coverage that a corrupt database does not prevent startup.
- Recovers profiles that are currently unlaunchable. The quarantined file is preserved, but its slot records are not carried over, so dead sessions in an affected profile lose cold-start recovery.
