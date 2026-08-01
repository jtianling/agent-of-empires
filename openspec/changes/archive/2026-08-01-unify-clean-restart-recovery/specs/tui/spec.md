## MODIFIED Requirements

### Requirement: C keybinding restarts agent panes clean
The TUI home screen SHALL support the `C` (Shift+C) keybinding as the state-aware action for starting the selected session's agents over without their previous conversations. When the tmux session exists, `C` SHALL restart every tracked agent pane with a fresh command that carries no resume flag, preserving the session layout. When the selected instance is recoverable because its tmux session does not exist but durable slots remain, `C` SHALL rebuild the session from those slots and launch every recovered pane fresh.

#### Scenario: C triggers a clean restart on a live session
- **WHEN** the user presses `C` on a selected session whose tmux session exists
- **THEN** the system SHALL initiate a fresh restart of the session's tracked agent panes
- **AND** the session layout SHALL be preserved
- **AND** no persisted resume token SHALL be passed to the relaunched commands

#### Scenario: C on a recoverable session triggers clean recovery
- **WHEN** the user presses `C` on a selected instance with durable slots whose tmux session does not exist
- **THEN** the system SHALL invoke cold recovery for that instance in fresh mode
- **AND** it SHALL rebuild the session and launch its persisted panes without any resume flag or token

#### Scenario: C on missing non-recoverable session
- **WHEN** the user presses `C` on a selected instance whose tmux session does not exist
- **AND** the instance has no durable slots
- **THEN** the system SHALL retain the existing single-pane fresh restart fallback

#### Scenario: C on session being deleted is a no-op
- **WHEN** the user presses `C` on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: C is shown in help and status hints
- **WHEN** the user opens the help overlay or views the home status bar for a selected session
- **THEN** the TUI SHALL list `C` as the clean restart action
- **AND** the contextual status hint SHALL indicate clean recovery when the selected instance is recoverable
