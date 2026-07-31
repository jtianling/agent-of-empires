## ADDED Requirements

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
