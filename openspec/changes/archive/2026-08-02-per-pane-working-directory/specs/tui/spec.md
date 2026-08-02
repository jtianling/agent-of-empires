## ADDED Requirements

### Requirement: Percent keybinding adds a managed agent pane

The TUI home screen SHALL support the `%` keybinding to add a managed agent pane to the selected session. `%` SHALL open a dialog offering the agent to launch, defaulting to the session's own tool, and the working directory to launch it in, defaulting to the session's own. On submit the system SHALL add the pane through the add-agent-pane action and then attach to the session.

`%` is chosen for its tmux meaning of "split to the right". AoE already binds `prefix + %` inside attached sessions to a split pinned to `@aoe_project_path`. The two are not the same action and their directory rules deliberately differ:

| Trigger | Result | Directory |
| --- | --- | --- |
| `prefix + %`, attached | raw tmux pane, no agent, no slot record, no identity key | forced to `@aoe_project_path` |
| `%`, home screen | managed agent pane: agent launched, slot recorded, identity key minted | chosen in the dialog, defaulting to the session's |

A `shell` pane added through `%` is slot-recorded only when it was given a directory of its own; one that inherited the session's stays slotless, per the `agent-session-store` capability. It is still a managed pane in every other respect: AoE launched it, and it is the dialog that made its directory a choice.

A hand-made split has no interface through which to name a directory, so inheriting the session's is the only useful behavior available to it. A managed pane is created through a dialog, which is such an interface. The distinction is whether the pane was given a chance to be configured, not whether the user was attached.

`%` SHALL remain a home-screen keybinding only. It SHALL NOT be added to the tmux key tables, so `prefix + %` keeps its existing meaning inside attached sessions.

#### Scenario: Percent adds a pane and attaches
- **WHEN** the user presses `%` on a selected running session and submits the dialog
- **THEN** the system SHALL add a managed agent pane to that session
- **AND** the TUI SHALL attach to the session

#### Scenario: The dialog defaults to the session's tool and directory
- **WHEN** the user presses `%` on a selected running session
- **THEN** the dialog SHALL preselect the session's own tool
- **AND** the working directory SHALL default to the session's own

#### Scenario: The dialog offers a different agent and directory
- **WHEN** the user presses `%` and chooses an agent other than the session's tool and a different working directory
- **THEN** the added pane SHALL run that agent in that directory

#### Scenario: Percent on a session that is not running
- **WHEN** the user presses `%` on a selected session whose tmux session does not exist
- **THEN** the system SHALL surface that the session is not running
- **AND** SHALL NOT start the session or create a pane

#### Scenario: Percent at the four-slot cap
- **WHEN** the user presses `%` on a selected session that already has four panes
- **THEN** the system SHALL surface that the cap is reached
- **AND** SHALL NOT create a pane

#### Scenario: Percent on a group row or a session being deleted is a no-op
- **WHEN** the user presses `%` with a group header selected, or on a session with status `Deleting`
- **THEN** the keybinding SHALL be a no-op

#### Scenario: Cancelling the dialog creates nothing
- **WHEN** the user presses `%` and cancels the dialog
- **THEN** no pane SHALL be created
- **AND** the TUI SHALL return to the home list without attaching

#### Scenario: Percent is shown in help
- **WHEN** the user opens the help overlay
- **THEN** the TUI SHALL list `%` as the action that adds an agent pane to the selected session
