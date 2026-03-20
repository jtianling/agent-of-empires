## ADDED Requirements

### Requirement: R keybinding restarts agent pane only
The TUI home screen SHALL support the `R` (Shift+R) keybinding to restart only the AoE-managed agent pane of the selected session, without destroying the session or its layout.

#### Scenario: R on session with dead agent pane
- **WHEN** the user presses `R` on a selected session
- **AND** the agent pane is dead
- **THEN** the system SHALL respawn the agent pane with the original agent command
- **AND** the session status SHALL transition to `Starting`
- **AND** the session layout SHALL be preserved

#### Scenario: R on session with running agent pane
- **WHEN** the user presses `R` on a selected session
- **AND** the agent pane is alive
- **THEN** the system SHALL force-restart the agent pane (kill + respawn)
- **AND** the session status SHALL transition to `Starting`

#### Scenario: R on session that does not exist
- **WHEN** the user presses `R` on a selected session
- **AND** the tmux session does not exist
- **THEN** the system SHALL start the session normally (same as attach behavior)

#### Scenario: R on session being deleted
- **WHEN** the user presses `R` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: R is shown in help overlay
- **WHEN** the user opens the help overlay (`?`)
- **THEN** the help SHALL list `R` as "Restart agent" or similar description
