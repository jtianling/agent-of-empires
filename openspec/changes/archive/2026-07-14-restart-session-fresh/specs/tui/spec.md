## MODIFIED Requirements

### Requirement: R keybinding restarts agent pane only
The TUI home screen SHALL support the `R` (Shift+R) keybinding to resume the AoE-managed agent panes of the selected session, without destroying the session or its layout. `R` SHALL keep its existing resume behavior (fan out to every tracked pane and resume each from its persisted `native_session_id`).

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
- **THEN** the help SHALL list `R` as "Resume agent panes" or similar description

## ADDED Requirements

### Requirement: r keybinding restarts agent panes fresh
The TUI home screen SHALL support the `r` (lowercase, no Shift) keybinding to restart every tracked agent pane of the selected session with a fresh command that carries no resume flag, preserving the session layout.

#### Scenario: r triggers a fresh restart on a session
- **WHEN** the user presses `r` on a selected session
- **THEN** the system SHALL initiate a fresh restart of the session's tracked agent panes
- **AND** the session layout SHALL be preserved

#### Scenario: r on session being deleted is a no-op
- **WHEN** the user presses `r` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: r is shown in help overlay
- **WHEN** the user opens the help overlay (`?`)
- **THEN** the help SHALL list `r` as "Restart agent panes (fresh)" or similar description

### Requirement: e keybinding opens the edit/rename dialog
The TUI home screen SHALL support the `e` keybinding to open the rename/edit dialog for the selected session, and the group-rename dialog when a group is selected.

#### Scenario: e opens the session rename dialog
- **WHEN** the user presses `e` on a selected session
- **THEN** the system SHALL open the session rename/edit dialog pre-filled with the session's current title, group, and profile

#### Scenario: e on session being deleted is a no-op
- **WHEN** the user presses `e` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: e is shown in help overlay
- **WHEN** the user opens the help overlay (`?`)
- **THEN** the help SHALL list `e` as "Edit/rename session" or similar description
