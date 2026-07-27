## Context

`aoe.db` is a per-profile SQLite store holding volatile pane captures, durable slot assignments, an append-only event stream, and one layout snapshot per instance. It is created by the migration system and applied idempotently by `ensure_schema` on every store open.

The incident that motivated this change: one profile's database reached 3.8 GB and its tail ended up 15 pages shorter than its header claimed, which SQLite reports as `database disk image is malformed`. AoE then failed to start in that profile.

Two separate things went wrong, and they are worth keeping apart.

**Growth.** `reconcile_session` appends an event for every assigned pane on every reconcile tick: `adopt` the first time a slot is recorded, `capture` every time after. The second branch fires on the status-poller cadence regardless of whether the capture changed, so a profile running several agents writes tens of rows per second forever. Nothing prunes the table, and `delete_slots_for_instance` removes slots and the layout snapshot but not events, so deleted sessions leave their rows behind permanently.

**Blast radius.** Every routine caller of the store (`reconcile_all`, `refresh_recoverable_cache`, the restart handlers) treats an open failure as `tracing::debug!` and continues. That is reasonable per-call, but it also means a corrupt database is invisible: the profile keeps running with capture and reconcile quietly doing nothing. The one path that does propagate is migration, which aborts startup on error. So the same corruption is either silent or fatal depending on which code happens to touch it first, and it was fatal only because a migration ran.

## Goals / Non-Goals

**Goals:**

- Stop the event stream from growing without bound.
- Make an unreadable store a recoverable condition rather than a launch failure.
- Tell the user when a database was quarantined, and where it went.
- Preserve the existing per-call tolerance of a store that cannot be opened.

**Non-Goals:**

- Repair a corrupt database or salvage rows from it.
- Add a general retention policy for the other tables. `pane_live` is already garbage-collected against live panes, `agent_slot` is bounded at four rows per instance, and `instance_layout` is one row per instance.
- Change what an event means or who reads it.
- Introduce a background vacuum or maintenance thread.

## Decisions

### Decision 1: Record capture events on change, not on tick

The reconciler appends a `capture` event only when the pane's captured session id differs from the one already on the slot. An unchanged capture writes the slot row (which refreshes `last_seen_at`) and appends nothing.

This is the actual fix. Retention alone would keep the file bounded while still writing tens of rows per second forever, and the rows carry no information: a stream of identical `capture` entries records that polling happened, not that anything occurred. `adopt` is already change-gated and stays as is.

### Decision 2: Retention is a backstop, applied on schema application

Pruning runs where the schema is applied, which is every store open, rather than on a timer or a background task. Two bounds apply together:

- rows older than a retention window are dropped, so an idle-but-old database shrinks;
- per instance, only the most recent N rows are kept, so one busy instance cannot crowd out the history of others.

Both are needed. Age alone lets a burst fill the window; count alone lets an old, quiet database keep rows forever.

This is deliberately a backstop rather than the primary mechanism. With Decision 1 in place the table should stay small on its own, and retention exists so that a future caller that appends carelessly cannot reproduce this incident.

### Decision 3: Deleting a session removes its events

`delete_slots_for_instance` already removes the instance's slots and layout snapshot; it now removes its events too. The current spec says event rows *may* be retained for history, which in practice means rows accumulate for sessions that no longer exist and can never be read in context.

### Decision 4: Quarantine and recreate, rather than fail or repair

When the schema cannot be applied because the database is corrupt or is not a database, the file is renamed with a timestamped suffix and an empty one is created in its place.

Repair is not attempted. `.recover` on a multi-gigabyte corrupt file is slow, its result is not trustworthy without inspection, and the contents are derived state: captures re-appear within a tick, slots are re-assigned from live panes, and only cold-start recovery for sessions that are *already dead* is genuinely lost. Trading that for a profile that cannot launch at all is not a real trade.

The file is renamed rather than deleted so the failure remains inspectable, and because the decision to discard several gigabytes of a user's data is not one this code should make silently.

Corruption is identified by SQLite's own result codes rather than by inspecting the file, so the check cannot drift from what actually fails to open.

### Decision 5: Quarantine is surfaced, not silent

A recovered profile looks identical to a healthy one, so without a message the user would learn nothing from an event that lost their recovery data and left a large file on disk. The warning names the quarantined path so it can be inspected or deleted deliberately.

This deliberately does not reuse the per-call tolerance described in Context: those callers stay quiet because a transient failure to open the store is not the user's problem, while a quarantine is a one-time, consequential event.

### Decision 6: Reclaim space after pruning

Deleting rows frees pages inside the file without returning them to the filesystem, so a database that has already grown would stay large. Space is reclaimed after a prune that actually removed rows, and only then, so a normal open does no extra work.

## Risks / Trade-offs

- [Quarantine discards slot records for dead sessions] -> Accepted and stated in the warning. Live sessions are unaffected; their slots are re-captured within a tick.
- [A transient open failure is misread as corruption] -> Only SQLite's corruption codes trigger a quarantine; lock contention, permissions, and missing files continue to surface as ordinary errors.
- [Reclaiming space on a large database blocks the open] -> It runs only after a prune that removed rows, which is a one-time cost on an already-oversized database.
- [Retention drops events someone is reading] -> Nothing in the codebase reads the event stream today; it exists for diagnostics. The window is chosen to keep recent history intact.
- [Change-gated capture events hide a stuck capture] -> `agent_slot.last_seen_at` still refreshes every tick, so liveness remains observable without an event per tick.

## Migration Plan

No schema change and no new migration. Pruning and quarantine take effect on the next store open. A profile that is currently unlaunchable recovers the first time the new binary runs in it.

## Open Questions

- None blocking. The retention window and per-instance cap are starting values; they can be tuned once the event stream is small enough to observe normally.
