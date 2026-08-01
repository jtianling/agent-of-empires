# pane-session-capture Specification

## Purpose
TBD - created by archiving change agent-session-recording. Update Purpose after archive.
## Requirements
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

### Requirement: Codex conversation binding covers every pane of a managed session

Codex rollout binding SHALL be attempted for each pane of a managed session, not for its primary pane alone. A Codex pane that is never bound produces no `pane_live` capture, therefore no durable slot, and is therefore skipped by restart and by cold-start recovery -- silently, because an untracked pane looks the same as a session that has none.

The preconditions that describe the instance rather than the pane SHALL be applied to the primary pane only. That the instance's tool is `codex`, and that its command has not been overridden, are statements about the instance's own agent pane; a non-primary pane may run a different agent than the instance's tool, and an override describes the program AoE launches for the instance, not what a user or a right-pane selection put in another pane. Every pane, primary or not, SHALL still be judged by the positive evidence already required: a process in its tree invoking Codex.

Panes SHALL be considered in ascending pane index, which is their creation order. Together with the existing rule that a thread id already held by another pane or slot is never claimed again, this SHALL keep a later-created pane from taking an earlier pane's conversation.

#### Scenario: A second Codex pane is bound to its own conversation
- **WHEN** a managed session has two panes each running Codex
- **AND** each has a rollout created after its own pane process started in the session's project path
- **THEN** the store SHALL hold a `pane_live` row for each pane
- **AND** the two rows SHALL carry different thread ids

#### Scenario: A bound extra pane reaches a durable slot
- **WHEN** a non-primary pane of a managed session has been bound to a conversation
- **AND** the reconciler runs
- **THEN** an `agent_slot` row SHALL exist for that pane

#### Scenario: A Codex pane in a session whose tool is not Codex is bound
- **WHEN** a managed session's instance tool is not `codex`
- **AND** one of its non-primary panes is running Codex
- **THEN** that pane SHALL be eligible for rollout binding

#### Scenario: A command override does not block a non-primary pane
- **WHEN** a managed session's instance carries a command override
- **AND** one of its non-primary panes is running Codex
- **THEN** that pane SHALL be eligible for rollout binding
- **AND** the instance's own primary pane SHALL still not be claimed for

#### Scenario: A non-primary pane not running Codex is not claimed for
- **WHEN** a non-primary pane of a managed session holds no process invoking Codex
- **THEN** no rollout SHALL be claimed for that pane

#### Scenario: The primary pane's conversation is not taken by a later pane
- **WHEN** a session's primary pane and a later-created pane are both eligible
- **AND** the primary pane's rollout is the earlier of the two
- **THEN** the primary pane SHALL hold that rollout's thread id
- **AND** the later pane SHALL hold a different one

### Requirement: A Codex pane's binding follows the process now in the pane

A Codex pane's recorded conversation SHALL describe the process currently running in that pane. AoE SHALL therefore attempt the rollout claim not only for a pane with no capture, but also for a pane whose capture was recorded before the process now in that pane started -- such a capture describes a process that no longer exists and cannot name the conversation now running.

This is the case a restart in place produces: restarting a live session respawns the pane and keeps its pane id, so the capture survives a change it does not describe. Restarting a session whose tmux is gone rebuilds it with fresh pane ids and so has no such capture.

The staleness comparison SHALL resolve ambiguity toward "not stale". A capture wrongly judged stale sends a correctly bound pane looking for another conversation; a stale capture wrongly judged fresh only defers the correction to the next reconcile.

A pane still running the conversation its capture records SHALL NOT be re-claimed for, however old that capture is. Resuming a pane respawns it, so its capture necessarily predates the new process while naming exactly the conversation now running; the resumed command line carries that conversation's id, which is direct evidence and SHALL outrank the timestamps.

All other conditions on the claim SHALL be unchanged: the pane's process tree must be invoking Codex, and the conversation must not already be claimed.

#### Scenario: A pane restarted in place is rebound to its new conversation

- **WHEN** a Codex pane holds a capture recorded before the process currently in that pane started
- **AND** the pane's process tree is invoking Codex
- **THEN** AoE SHALL attempt the rollout claim for that pane
- **AND** the pane's capture SHALL name the conversation the current process is running

#### Scenario: A live binding is not disturbed

- **WHEN** a Codex pane holds a capture recorded after the process currently in that pane started
- **THEN** AoE SHALL NOT attempt the rollout claim for that pane
- **AND** the existing capture SHALL be left unchanged

#### Scenario: A resumed pane keeps its conversation

- **WHEN** a Codex pane is resumed, so its capture predates the process now in it
- **AND** that process's command line names the conversation the capture records
- **THEN** AoE SHALL NOT attempt the rollout claim for that pane
- **AND** the pane SHALL keep its existing conversation

#### Scenario: A restarted pane's conversation is not taken by a sibling

- **WHEN** a managed session has two Codex panes and one of them is restarted in place
- **THEN** the restarted pane SHALL be bound to the conversation its own process is running
- **AND** the sibling pane's binding SHALL be unchanged

### Requirement: Only conversations that could have run in a pane are eligible

A conversation that a non-interactive Codex run produced SHALL NOT be bound to a pane. AoE SHALL read the rollout's `originator` and skip a rollout whose originator names a known non-interactive Codex entry point, regardless of how well it matches on working directory and time.

The filter SHALL be expressed as a set of originators to reject rather than a set to accept. Rejecting an unrecognized originator would stop every pane from being adopted if a future Codex release renamed the interactive one, and an unadopted pane has no slot, so it is silently excluded from restart and from recovery -- the same failure this requirement exists to prevent. Accepting an unrecognized originator leaves the prior behavior in place for it instead.

An originator that is neither known-interactive nor known-non-interactive SHALL be accepted and reported, so that a change in Codex's values is visible rather than silently absorbed.

#### Scenario: A scripted Codex run is not bound to a pane

- **WHEN** a rollout in the pane's working directory was written by a non-interactive Codex run
- **AND** it would otherwise be the earliest unclaimed match for a pane
- **THEN** AoE SHALL skip it
- **AND** the pane SHALL be bound to an interactive conversation, or to none

#### Scenario: An unrecognized originator is accepted and reported

- **WHEN** a rollout carries an originator that is neither known-interactive nor known-non-interactive
- **THEN** AoE SHALL still consider it eligible
- **AND** AoE SHALL report the unrecognized originator

#### Scenario: A rollout without an originator remains eligible

- **WHEN** a rollout records no originator
- **THEN** AoE SHALL consider it eligible, as it did before this filter existed

