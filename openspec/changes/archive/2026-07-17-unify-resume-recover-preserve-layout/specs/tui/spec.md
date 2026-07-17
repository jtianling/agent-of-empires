## MODIFIED Requirements

### Requirement: R keybinding resumes or recovers the selected session
The TUI home screen SHALL support the `R` (Shift+R) keybinding as the state-aware action for returning to the selected session's persisted conversations. When the tmux session exists, `R` SHALL resume every tracked pane from its persisted `native_session_id` without changing the layout. When the selected instance is recoverable because its tmux session does not exist but durable slots remain, `R` SHALL rebuild and recover it from those slots.

#### Scenario: R on session with dead agent pane
- **WHEN** the user presses `R` on a selected session whose tmux session exists
- **AND** an agent pane is dead
- **THEN** the system SHALL respawn every tracked agent pane through resume mode
- **AND** the session status SHALL transition to `Starting`
- **AND** the session layout SHALL be preserved

#### Scenario: R on session with running agent pane
- **WHEN** the user presses `R` on a selected session whose tmux session exists
- **AND** an agent pane is alive
- **THEN** the system SHALL force-restart every tracked agent pane in resume mode
- **AND** the session status SHALL transition to `Starting`

#### Scenario: R on recoverable session
- **WHEN** the user presses `R` on a selected instance with durable slots whose tmux session does not exist
- **THEN** the system SHALL invoke cold recovery for that instance
- **AND** it SHALL rebuild the session and resume its persisted panes

#### Scenario: R on missing non-recoverable session
- **WHEN** the user presses `R` on a selected instance whose tmux session does not exist
- **AND** the instance has no durable slots
- **THEN** the system SHALL retain the existing normal-start fallback behavior

#### Scenario: R on session being deleted
- **WHEN** the user presses `R` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: R is shown contextually
- **WHEN** the selected instance is recoverable
- **THEN** the home status bar SHALL show `R` as the recover action
- **AND** the help overlay SHALL describe `R` as the resume/recover action

#### Scenario: R is shown for a live session
- **WHEN** the selected instance is not recoverable
- **THEN** the home status bar SHALL show `R` as the resume action

## ADDED Requirements

### Requirement: C keybinding restarts agent panes clean
The TUI home screen SHALL support the `C` (Shift+C) keybinding to restart every tracked agent pane of the selected live session with a fresh command that carries no resume flag, preserving the session layout.

#### Scenario: C triggers a clean restart on a live session
- **WHEN** the user presses `C` on a selected session whose tmux session exists
- **THEN** the system SHALL initiate a fresh restart of the session's tracked agent panes
- **AND** the session layout SHALL be preserved
- **AND** no persisted resume token SHALL be passed to the relaunched commands

#### Scenario: C on session being deleted is a no-op
- **WHEN** the user presses `C` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: C is shown in help and status hints
- **WHEN** the user opens the help overlay or views the home status bar for a selected live session
- **THEN** the TUI SHALL list `C` as the clean or fresh restart action

## REMOVED Requirements

### Requirement: r keybinding restarts agent panes fresh
**Reason**: Lowercase `r` differs from resume only by Shift state and makes the destructive fresh action too easy to confuse with resume.

**Migration**: Users SHALL press `Shift+C` for a fresh restart. `Shift+R` remains the resume/recover action.

### Requirement: Separate V recovery keybinding
**Reason**: Cold recovery and live resume express the same user intent and are now routed contextually through `Shift+R`.

**Migration**: Users SHALL select a recoverable instance and press `Shift+R`; `Shift+V` no longer triggers recovery.
