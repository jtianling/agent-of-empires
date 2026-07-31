## MODIFIED Requirements

### Requirement: Optional add-agent-pane action
The system SHALL provide an explicit action to add an agent pane to the current session (splitting the tmux window and launching an agent). This action is optional for the user (observation/adoption remains the primary path) and SHALL respect the four-slot cap.

The pane it creates is not the instance's own agent pane. Its command SHALL be built as a non-primary pane: the instance's command override, pre-allocated session id, fork token, and identity key describe the instance's own agent and SHALL NOT be applied to it. Reusing the instance's identity key in particular would put two live panes behind one identity, which is the one state the identity design cannot recover from.

#### Scenario: Add-agent-pane creates and tracks a new pane
- **WHEN** the user triggers the add-agent-pane action on a session with fewer than four tracked panes
- **THEN** the system SHALL create a new tmux pane in that session running an agent
- **AND** the new pane SHALL be eligible for adoption into a slot

#### Scenario: Add-agent-pane blocked at the cap
- **WHEN** the user triggers the add-agent-pane action on a session that already tracks four panes
- **THEN** the system SHALL NOT create a fifth agent pane
- **AND** SHALL surface that the four-slot cap is reached

#### Scenario: Added pane does not present the instance's identity
- **WHEN** the add-agent-pane action runs on a Cross Agent Team session whose instance holds an identity key
- **THEN** the added pane's environment SHALL NOT contain that key

#### Scenario: Added pane does not inherit the instance's conversation
- **WHEN** the add-agent-pane action runs on a session with a pre-allocated session id or a command override
- **THEN** the added pane's command SHALL NOT contain either of them
