## MODIFIED Requirements

### Requirement: Optional add-agent-pane action

The system SHALL provide an explicit action to add an agent pane to the current session (splitting the tmux window and launching an agent). This action is optional for the user (observation/adoption remains the primary path) and SHALL respect the four-slot cap.

The action SHALL be reachable from the TUI and from the CLI. Both entry points SHALL accept the agent to launch and the working directory to launch it in. Neither SHALL be fixed to the session's own: the pane is a peer of the pane beside it, and an entry point that can only reproduce the session's tool in the session's directory cannot express what the session was created to do.

An unspecified agent SHALL default to the session's own tool. An unspecified working directory SHALL default to the session's own, resolved when the pane is split.

The action SHALL refuse a session that is not running rather than starting it. Starting a session is a larger act than the action promises, and the pane it would add is not the pane a start produces.

The pane it creates is not the instance's own agent pane. Its command SHALL be built as a non-primary pane: the instance's command override, pre-allocated session id, fork token, and identity key describe the instance's own agent and SHALL NOT be applied to it. Reusing the instance's identity key in particular would put two live panes behind one identity, which is the one state the identity design cannot recover from.

#### Scenario: Add-agent-pane creates and tracks a new pane
- **WHEN** the user triggers the add-agent-pane action on a session with fewer than four tracked panes
- **THEN** the system SHALL create a new tmux pane in that session running an agent
- **AND** the new pane SHALL be eligible for adoption into a slot

#### Scenario: Add-agent-pane blocked at the cap
- **WHEN** the user triggers the add-agent-pane action on a session that already tracks four panes
- **THEN** the system SHALL NOT create a fifth agent pane
- **AND** SHALL surface that the four-slot cap is reached

#### Scenario: Add-agent-pane launches the requested agent
- **WHEN** the user triggers the add-agent-pane action and names an agent other than the session's tool
- **THEN** the created pane SHALL run that agent
- **AND** the session's own tool SHALL be unchanged

#### Scenario: Add-agent-pane launches in the requested directory
- **WHEN** the user triggers the add-agent-pane action and names a working directory
- **THEN** the created pane SHALL start in that directory
- **AND** that directory SHALL be recorded on the pane's durable slot

#### Scenario: Add-agent-pane defaults to the session's tool and directory
- **WHEN** the user triggers the add-agent-pane action without naming an agent or a directory
- **THEN** the created pane SHALL run the session's own tool in the session's own working directory

#### Scenario: Add-agent-pane refuses a session that is not running
- **WHEN** the user triggers the add-agent-pane action on a session whose tmux session does not exist
- **THEN** no pane SHALL be created
- **AND** the system SHALL surface that the session is not running
- **AND** the session SHALL NOT be started

#### Scenario: Added pane does not present the instance's identity
- **WHEN** the add-agent-pane action runs on a Cross Agent Team session whose instance holds an identity key
- **THEN** the added pane's environment SHALL NOT contain that key

#### Scenario: Added pane does not inherit the instance's conversation
- **WHEN** the add-agent-pane action runs on a session with a pre-allocated session id or a command override
- **THEN** the added pane's command SHALL NOT contain either of them
