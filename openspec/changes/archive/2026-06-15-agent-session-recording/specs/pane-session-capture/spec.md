## ADDED Requirements

### Requirement: Hook captures native session id keyed by tmux pane
The installed agent status hook SHALL, in addition to its existing status-file write, capture the agent's native session id into the SQLite store keyed by `$TMUX_PANE`. The native session id SHALL be read from the hook's **stdin JSON** (`.session_id`), not from a `$CLAUDE_SESSION_ID` (or similar) environment variable. The capture SHALL also record the working directory (`.cwd` from stdin or `$PWD`). The legacy environment-variable session-id capture SHALL be removed.

#### Scenario: Claude session id captured from stdin
- **WHEN** a Claude agent fires a hook event inside a tmux pane
- **AND** the hook stdin JSON contains `session_id`
- **THEN** the store SHALL hold a `pane_live` row for that pane's `$TMUX_PANE`
- **AND** the row's `native_session_id` SHALL equal the stdin `session_id`
- **AND** the row's `cwd` SHALL equal the agent's working directory

#### Scenario: Hand-launched agent without AOE_INSTANCE_ID is still captured
- **WHEN** a user manually runs an agent inside a shell pane (no `$AOE_INSTANCE_ID` in the environment)
- **AND** the pane has a `$TMUX_PANE` value
- **THEN** the hook SHALL still write the `pane_live` capture row
- **AND** the capture SHALL NOT depend on `$AOE_INSTANCE_ID`

#### Scenario: Capture no-ops outside tmux
- **WHEN** an agent fires a hook event but `$TMUX_PANE` is empty (not running inside tmux)
- **THEN** the hook SHALL NOT write a capture row
- **AND** the hook SHALL exit successfully without error

### Requirement: Reconciler snapshots pane captures into durable slots
The system SHALL run a reconciler on the existing status-poller tick. For each managed session, the reconciler SHALL enumerate the session's tmux panes, resolve each pane's capture via `pane_live` keyed by `$TMUX_PANE`, and upsert a durable `agent_slot` record `(instance_id, slot, agent, native_session_id, cwd, tmux_pane, last_seen_at)`. The primary `@aoe_agent_pane` SHALL be slot 0. Assignment SHALL be sticky: a pane that already owns a slot keeps it, so a newly appearing pane SHALL NOT evict an already-tracked pane. New panes SHALL fill remaining free slots in ascending pane-index order, capped at 4 panes per session.

#### Scenario: Already-tracked pane keeps its slot when a new pane appears
- **WHEN** a session already has four panes recorded in `agent_slot` (slots 0..3)
- **AND** a new pane appears, even with a lower pane index than an existing pane
- **AND** the reconciler tick runs
- **THEN** each already-tracked pane SHALL retain its original slot
- **AND** the new pane SHALL NOT be recorded (no fifth slot, no eviction)

#### Scenario: Managed session pane snapshotted to a slot
- **WHEN** a managed session has a pane whose `pane_live` capture has a `native_session_id`
- **AND** the reconciler tick runs
- **THEN** an `agent_slot` row SHALL exist for that `(instance_id, slot)` with the captured `native_session_id`
- **AND** `last_seen_at` SHALL be updated to the reconcile time

#### Scenario: At most four slots per session
- **WHEN** a managed session has more than four panes running agents
- **THEN** the reconciler SHALL record at most four `agent_slot` rows for that session

#### Scenario: Orphan captures are garbage-collected
- **WHEN** `pane_live` holds a row whose `tmux_pane` does not belong to any currently managed session
- **AND** the reconciler tick runs
- **THEN** that orphan `pane_live` row SHALL be removed

#### Scenario: Snapshot occurs while tmux is alive
- **WHEN** a managed session's agent has an active capture
- **THEN** the reconciler SHALL snapshot it into `agent_slot` during normal ticks (before any teardown)
- **AND** the durable record SHALL therefore be available even after the tmux session no longer exists
