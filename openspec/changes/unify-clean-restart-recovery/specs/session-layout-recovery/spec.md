## MODIFIED Requirements

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
