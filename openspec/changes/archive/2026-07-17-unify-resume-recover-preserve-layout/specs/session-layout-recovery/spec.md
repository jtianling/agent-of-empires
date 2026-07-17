## ADDED Requirements

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

When recovering an instance with a valid coherent layout snapshot, the system SHALL recreate one pane per persisted slot and apply the saved horizontal and vertical split topology to the new panes before attaching the user. Each saved pane leaf SHALL be validated against its previous `agent_slot.tmux_pane`, and the recovered tmux pane-list order SHALL align durable slot order with the saved layout's leaf order.

#### Scenario: Right column vertical split is restored
- **WHEN** a recoverable three-pane instance was last saved as one left pane and two vertically stacked panes on the right
- **AND** the user invokes recovery
- **THEN** the rebuilt tmux window SHALL have one left pane and a vertically split right column
- **AND** it SHALL NOT be flattened into three horizontal columns

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
