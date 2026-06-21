## MODIFIED Requirements

### Requirement: Reconciler snapshots pane captures into durable slots
The system SHALL run a reconciler that snapshots pane captures into durable slots, driven from at least two attach-independent sources: the existing TUI status-poller tick AND the long-lived notification-monitor process (`aoe tmux monitor-notifications`). The reconciler SHALL continue to advance `agent_slot` while the TUI is attached to a session -- that is, while the status poller is not ticking because the main loop is blocked on `tmux attach-session`. For each managed session, the reconciler SHALL enumerate the session's tmux panes, resolve each pane's capture via `pane_live` keyed by `$TMUX_PANE`, and upsert a durable `agent_slot` record `(instance_id, slot, agent, native_session_id, cwd, tmux_pane, last_seen_at)`. The primary `@aoe_agent_pane` SHALL be slot 0. Assignment SHALL be sticky: a pane that already owns a slot keeps it, so a newly appearing pane SHALL NOT evict an already-tracked pane. New panes SHALL fill remaining free slots in ascending pane-index order, capped at 4 panes per session. The reconciler is idempotent, so running it from multiple drivers SHALL NOT create duplicate or conflicting rows. The notification-monitor driver SHALL be throttled by a minimum interval so it does not query tmux on every short poll cycle.

#### Scenario: Reconcile continues while attached to a session
- **WHEN** the TUI is attached to a managed session (the status poller is not ticking)
- **AND** a pane in that session produces a new `pane_live` capture with a `native_session_id`
- **THEN** the reconciler SHALL still run from the notification-monitor process
- **AND** an `agent_slot` row SHALL reflect that capture within a bounded time, without the user returning to the home view

#### Scenario: Already-tracked pane keeps its slot when a new pane appears
- **WHEN** a session already has four panes recorded in `agent_slot` (slots 0..3)
- **AND** a new pane appears, even with a lower pane index than an existing pane
- **AND** the reconciler runs
- **THEN** each already-tracked pane SHALL retain its original slot
- **AND** the new pane SHALL NOT be recorded (no fifth slot, no eviction)

#### Scenario: Managed session pane snapshotted to a slot
- **WHEN** a managed session has a pane whose `pane_live` capture has a `native_session_id`
- **AND** the reconciler runs
- **THEN** an `agent_slot` row SHALL exist for that `(instance_id, slot)` with the captured `native_session_id`
- **AND** `last_seen_at` SHALL be updated to the reconcile time

#### Scenario: At most four slots per session
- **WHEN** a managed session has more than four panes running agents
- **THEN** the reconciler SHALL record at most four `agent_slot` rows for that session

#### Scenario: Orphan captures are garbage-collected
- **WHEN** `pane_live` holds a row whose `tmux_pane` does not belong to any currently managed session
- **AND** the reconciler runs
- **THEN** that orphan `pane_live` row SHALL be removed

#### Scenario: Snapshot occurs while tmux is alive
- **WHEN** a managed session's agent has an active capture
- **THEN** the reconciler SHALL snapshot it into `agent_slot` during normal runs (before any teardown)
- **AND** the durable record SHALL therefore be available even after the tmux session no longer exists
