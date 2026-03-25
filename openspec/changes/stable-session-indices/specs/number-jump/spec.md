## MODIFIED Requirements

### Requirement: Global numeric index assignment
The system SHALL assign 1-based numeric indices to all sessions in the TUI list, following the display order computed from the fully-expanded group tree (all groups treated as expanded). Group headers SHALL NOT receive indices. Sessions inside collapsed groups SHALL still receive stable indices computed from the expanded tree, but those indices SHALL NOT be displayed in the TUI when their group is collapsed. Indices SHALL be recalculated on every render. Maximum index is 99.

#### Scenario: Simple flat list numbering
- **WHEN** the session list contains 5 ungrouped sessions
- **THEN** sessions SHALL be numbered 1 through 5 in display order

#### Scenario: Grouped list numbering skips group headers
- **WHEN** the session list contains a group "backend" with 3 sessions and a group "frontend" with 2 sessions
- **THEN** the group headers SHALL have no number
- **AND** sessions SHALL be numbered 1 through 5 consecutively across groups

#### Scenario: Collapsed group sessions retain stable indices
- **WHEN** a group containing sessions 3, 4, 5 is collapsed
- **THEN** those 3 sessions SHALL retain indices 3, 4, 5
- **AND** sessions after the collapsed group SHALL keep their original indices (no renumbering)
- **AND** the TUI SHALL NOT display the indices of hidden sessions

#### Scenario: Expanding a collapsed group shows original indices
- **WHEN** a group was collapsed (hiding sessions 3, 4, 5)
- **AND** the user expands the group
- **THEN** sessions 3, 4, 5 SHALL appear with the same indices they had before collapse

#### Scenario: More than 99 sessions
- **WHEN** there are more than 99 visible sessions
- **THEN** only the first 99 SHALL receive numeric indices
- **AND** sessions 100+ SHALL have no number displayed

### Requirement: CLI switch-session supports --index parameter
The `aoe tmux switch-session` command SHALL accept an `--index N` parameter (1-based) as an alternative to `--direction`. The index resolves against the global ordered session list computed from the fully-expanded group tree (same order as stable TUI indices), not scoped to the current group. Upon successful switch, the system SHALL set `@aoe_index` on the target session to its 1-based position in the ordered list.

#### Scenario: Switch by index
- **WHEN** `aoe tmux switch-session --index 3 --profile default` is called
- **THEN** the system SHALL switch to the 3rd session in the global stable display order

#### Scenario: Index out of range
- **WHEN** `aoe tmux switch-session --index 50` is called
- **AND** there are only 10 sessions
- **THEN** no switch SHALL occur
- **AND** the command SHALL exit successfully (no error)

#### Scenario: Index resolves at runtime
- **WHEN** sessions are created or deleted between the time bindings were set and the jump is triggered
- **THEN** the index SHALL resolve against the current session list at the time of the jump

#### Scenario: @aoe_index set on target after switch
- **WHEN** `aoe tmux switch-session --index 3` is called and the switch succeeds
- **THEN** the target session SHALL have `@aoe_index` set to its current 1-based position

#### Scenario: Index stable across collapse/expand
- **WHEN** a group is collapsed and the user presses `Ctrl+b 5 Space`
- **AND** session #5 exists but is inside the collapsed group
- **THEN** the system SHALL switch to session #5
