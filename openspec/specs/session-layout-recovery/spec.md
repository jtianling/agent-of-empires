# Capability Spec: Session Layout Recovery

**Capability**: `session-layout-recovery`
**Created**: 2026-07-17
**Status**: Stable

## Purpose

Preserve the topology and spatial ownership of panes in AoE-managed tmux sessions so durable agent conversations can be recovered after the tmux session or machine restarts.
## Requirements
### Requirement: Coherent session layout snapshots are persisted

While an AoE-managed tmux session is alive, the system SHALL persist a recent serialized window layout for the instance only when the layout pane set maps one-to-one to that instance's durable `agent_slot` pane set. The snapshot SHALL be keyed by instance and SHALL survive application and machine restarts.

#### Scenario: Nested tracked layout is captured
- **WHEN** a live instance has three tracked panes arranged as one left pane and a vertically split right column
- **AND** the live layout pane ids match the instance's durable slot pane ids
- **THEN** the system SHALL persist the serialized nested layout for that instance

#### Scenario: Partial pane set does not replace the snapshot
- **WHEN** a reconcile observation contains a layout whose pane ids do not map one-to-one to the durable slots
- **THEN** the system SHALL NOT replace the instance's last coherent layout snapshot with that observation

#### Scenario: Layout snapshot survives restart
- **WHEN** AoE exits or the machine restarts after a coherent layout was persisted
- **THEN** the layout snapshot SHALL remain available with the instance's durable recovery records

### Requirement: Cold recovery restores saved pane topology

When recovering an instance with a valid coherent layout snapshot, the system SHALL recreate one pane per persisted slot and apply the saved horizontal and vertical split topology to the new panes before attaching the user. Each saved pane leaf SHALL be validated against its previous `agent_slot.tmux_pane`, and the recovered tmux pane-list order SHALL align durable slot order with the saved layout's leaf order. Topology restoration SHALL be independent of the restart mode: it applies identically whether the recovered panes resume their previous conversations or launch fresh.

#### Scenario: Right column vertical split is restored
- **WHEN** a recoverable three-pane instance was last saved as one left pane and two vertically stacked panes on the right
- **AND** the user invokes recovery
- **THEN** the rebuilt tmux window SHALL have one left pane and a vertically split right column
- **AND** it SHALL NOT be flattened into three horizontal columns

#### Scenario: Topology is restored for clean recovery
- **WHEN** a recoverable multi-pane instance with a nested saved layout is recovered in fresh mode
- **THEN** the rebuilt tmux window SHALL have the saved split topology
- **AND** each slot SHALL occupy the same spatial cell that its previous pane occupied
- **AND** no pane SHALL be launched with a resume flag

#### Scenario: Slot ownership survives layout application
- **WHEN** the stored layout is remapped and applied to newly created panes
- **THEN** each pane SHALL resume the agent and native session id belonging to its original durable slot
- **AND** each slot's `tmux_pane` SHALL be updated to that slot's new pane id
- **AND** each slot SHALL occupy the same spatial cell that its previous pane occupied

#### Scenario: Geometry scales to the recovered window
- **WHEN** the recovered tmux window dimensions differ from the dimensions recorded in the snapshot
- **THEN** the system SHALL preserve the saved split topology
- **AND** tmux MAY scale pane dimensions to fit the current window

### Requirement: Invalid or unavailable layout degrades safely

Layout restoration SHALL be best-effort and SHALL NOT prevent recovery of persisted conversations. If the snapshot is absent, malformed, stale, does not map exactly to the recovered slots, or cannot be applied by tmux, the system SHALL retain a valid fallback pane arrangement and continue per-pane resume recovery.

#### Scenario: Existing profile has no layout snapshot
- **WHEN** an instance created before layout persistence is recovered
- **THEN** the system SHALL rebuild and resume its durable panes using the fallback layout
- **AND** recovery SHALL NOT fail solely because no layout snapshot exists

#### Scenario: Stored layout references a missing pane
- **WHEN** a saved layout leaf cannot be mapped from an old slot pane id to a newly created pane id
- **THEN** the system SHALL NOT apply the partial layout
- **AND** it SHALL continue recovering every recoverable pane

#### Scenario: tmux rejects the remapped layout
- **WHEN** tmux returns an error while applying an otherwise validated remapped layout
- **THEN** the system SHALL preserve the fallback pane arrangement
- **AND** it SHALL surface or log a layout-specific warning without aborting sibling pane recovery

### Requirement: Layout persistence follows instance record lifecycle

The system SHALL keep at most one current layout snapshot per instance and SHALL remove that snapshot when the instance's durable session records are removed.

#### Scenario: New coherent snapshot replaces the old snapshot
- **WHEN** a tracked session is resized or re-split and a new coherent layout is observed
- **THEN** the stored snapshot for that instance SHALL be updated to the new layout

#### Scenario: Deleting session records removes layout
- **WHEN** the durable records for an instance are deleted
- **THEN** its stored layout snapshot SHALL also be deleted

### Requirement: Recovery launches each agent exactly once

When rebuilding a session during recovery, the system SHALL create it with a placeholder shell rather than an agent command, so that every recovered pane's agent is launched once, by the per-slot launch that determines what that slot runs.

Rebuilding SHALL still perform the rest of the session setup recovery depends on: worktree and sandbox preparation, on-launch hooks, and tmux options.

A placeholder rebuild SHALL NOT clear a pending fork token, because no agent has been launched to consume it, and SHALL NOT run agent startup auto-confirmation, because the pane holds a shell.

#### Scenario: Rebuilt session survives an agent that refuses to start

- **WHEN** a recoverable instance is recovered
- **AND** its agent exits immediately when launched
- **THEN** the rebuilt tmux session SHALL still exist
- **AND** each durable slot's pane SHALL have been respawned with that slot's command
- **AND** recovery SHALL NOT report that a pane could not be found

#### Scenario: Both restart modes rebuild the same way
- **WHEN** an instance is recovered in resume mode or in fresh mode
- **THEN** the session rebuild SHALL create a placeholder in both cases
- **AND** the per-slot launch SHALL remain the only place the agent command is run

#### Scenario: Pending fork survives a placeholder rebuild
- **WHEN** an instance with a pending fork token is recovered
- **THEN** the rebuild SHALL leave the token in place for the per-slot launch

### Requirement: A pane being relaunched survives its own process kill

Relaunching a tracked pane SHALL hold that pane open across the process-tree kill it performs, regardless of how the pane was created, and SHALL then set `remain-on-exit` to the value the newly launched agent requires: held open for an agent, closing on exit for a plain shell.

The kill happens outside tmux because an agent's children can outlive the signal a tmux-internal respawn sends them, and tmux destroys a pane whose `remain-on-exit` is off as soon as its process goes -- which would leave the respawn with no pane to target.

The setting SHALL always be written rather than left to whatever the pane last carried, since a pane can be relaunched as an agent after having been created as a shell, or the reverse.

#### Scenario: The only slot of a shell-command instance comes back
- **WHEN** an instance whose command is a shell has a single slot recording an agent
- **AND** that instance is recovered from a cold start
- **THEN** the slot's pane SHALL still exist after the relaunch, running the agent the slot recorded
- **AND** the session SHALL NOT be destroyed by the relaunch

#### Scenario: A relaunched shell pane still closes when it exits
- **WHEN** a slot recording a plain shell is relaunched
- **THEN** its pane SHALL be left closing on exit rather than held open

### Requirement: Recovery reports slots that did not come back

After launching every slot and applying the saved layout, recovery SHALL verify that each durable slot has a live pane in the rebuilt session, and SHALL report each slot that does not as a per-pane failure.

The verification SHALL happen after a brief settle rather than immediately, because a pane can survive its own relaunch and disappear shortly afterwards. The report SHALL identify the slot by the agent and working directory it recorded, which is what the user recognizes, rather than only by a pane id that no longer exists.

Recovery SHALL NOT retry or repair a missing pane; reporting is the required behavior.

#### Scenario: A slot whose pane disappears is reported
- **WHEN** a recovered slot's pane is launched successfully and then disappears
- **THEN** recovery SHALL report that slot as failed
- **AND** the report SHALL name the agent and working directory the slot recorded

#### Scenario: Recovery with every pane present reports no failure
- **WHEN** every durable slot has a live pane after the rebuild
- **THEN** recovery SHALL report no per-pane failure

#### Scenario: A missing pane is not silently repaired
- **WHEN** a recovered slot has no live pane
- **THEN** recovery SHALL NOT relaunch or recreate it

