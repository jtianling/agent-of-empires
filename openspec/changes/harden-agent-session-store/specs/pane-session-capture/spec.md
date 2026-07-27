## MODIFIED Requirements

### Requirement: Reconciler snapshots pane captures into durable slots
The reconciler SHALL, per managed session, enumerate the session's tmux panes, resolve each pane's capture from `pane_live`, assign a deterministic slot, and upsert an `agent_slot` row.

It SHALL append an `adopt` event the first time a slot is recorded for a session, and a `capture` event only when the pane's captured native session id differs from the one already recorded on that slot. An unchanged capture SHALL still refresh the durable row, so liveness stays observable through `last_seen_at`, but SHALL NOT append an event. The reconciler runs on the poll cadence, so appending per tick would record that polling happened rather than that anything occurred.

#### Scenario: Pane capture is snapshotted into a slot
- **WHEN** a managed session has a pane with a recorded capture
- **THEN** the reconciler SHALL upsert an `agent_slot` row for that pane's slot

#### Scenario: First recording of a slot appends adopt
- **WHEN** a pane is assigned a slot that was not previously recorded for the session
- **THEN** the reconciler SHALL append an `adopt` event for that slot

#### Scenario: Changed capture appends one event
- **WHEN** a tracked slot's pane reports a native session id different from the one recorded
- **THEN** the reconciler SHALL append a `capture` event for that slot

#### Scenario: Unchanged capture appends nothing
- **WHEN** a tracked slot's pane reports the same native session id already recorded
- **THEN** the reconciler SHALL NOT append an event
- **AND** the durable row's `last_seen_at` SHALL still be refreshed
