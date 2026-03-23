## ADDED Requirements

### Requirement: Ctrl+Tab switches to last-active pane
When a user is in an AoE-managed tmux session with multiple panes, pressing `Ctrl+b` followed by `Ctrl+Tab` SHALL execute tmux `last-pane`, switching focus to the previously active pane in the current window.

#### Scenario: Toggle between two panes in a split window
- **WHEN** the user has a left/right split with pane A (active) and pane B (last-active)
- **AND** the user presses `Ctrl+b Ctrl+Tab`
- **THEN** focus SHALL move to pane B

#### Scenario: Repeated toggle returns to original pane
- **WHEN** the user presses `Ctrl+b Ctrl+Tab` twice
- **THEN** focus SHALL return to the original pane

#### Scenario: Single pane window (no-op)
- **WHEN** the window has only one pane
- **AND** the user presses `Ctrl+b Ctrl+Tab`
- **THEN** nothing SHALL happen (tmux `last-pane` is a no-op with one pane)

### Requirement: Binding follows keybinding lifecycle
The `Ctrl+Tab` binding SHALL be registered in `setup_session_cycle_bindings()` and cleaned up in `cleanup_session_cycle_bindings()`, following the same lifecycle as existing pane navigation bindings (`h`, `j`, `k`, `l`).

#### Scenario: Binding available after session attach
- **WHEN** a user attaches to an AoE-managed session (nested or non-nested)
- **THEN** `Ctrl+b Ctrl+Tab` SHALL be bound to `last-pane`

#### Scenario: Binding cleaned up on detach
- **WHEN** a user detaches from an AoE-managed session
- **THEN** the `Ctrl+Tab` binding SHALL be removed from the prefix table

### Requirement: Works in both nested and non-nested modes
The binding SHALL function identically in both nested mode (AoE running inside an existing tmux session) and non-nested mode (AoE started from a bare terminal).

#### Scenario: Non-nested mode pane toggle
- **WHEN** AoE is started from a bare terminal (no `TMUX` env var)
- **AND** the user presses `Ctrl+b Ctrl+Tab`
- **THEN** `last-pane` SHALL execute

#### Scenario: Nested mode pane toggle
- **WHEN** AoE is started inside an existing tmux session (`TMUX` env var is set)
- **AND** the user presses `Ctrl+b Ctrl+Tab`
- **THEN** `last-pane` SHALL execute
