## ADDED Requirements

### Requirement: A session tracks up to four agent slots
A managed session SHALL be able to track up to four agent panes, each represented by a slot (0..3) in the durable store. This is additive to the existing single primary managed pane: the primary pane occupies a slot, and additional tracked panes occupy further slots. The system SHALL NOT track more than four agent panes per session.

#### Scenario: Primary pane occupies a slot
- **WHEN** a session is started with its primary managed agent
- **THEN** that agent SHALL be tracked as one of the session's slots

#### Scenario: Tracking caps at four
- **WHEN** a session already tracks four agent panes
- **AND** a fifth agent pane appears
- **THEN** the system SHALL NOT create a fifth slot for that session
- **AND** the existing four slots SHALL remain unchanged

### Requirement: Agents appearing in any pane are adopted
The system SHALL adopt (begin tracking) an agent that appears in any pane of a managed session, regardless of whether AoE launched it or the user started it by hand. Adoption SHALL be observe-first: it does not require the user to pre-declare the pane.

#### Scenario: Agent in a user-created split pane is adopted
- **WHEN** a managed session has a user-created split pane
- **AND** the user runs an agent in that pane
- **AND** the agent produces a capture (native session id)
- **THEN** the system SHALL assign that pane a slot and record it in `agent_slot`

#### Scenario: Adoption recorded as an event
- **WHEN** a previously untracked pane is adopted into a slot
- **THEN** the system SHALL append an `adopt` event for that `(instance_id, slot)`

### Requirement: Optional add-agent-pane action
The system SHALL provide an explicit action to add an agent pane to the current session (splitting the tmux window and launching an agent). This action is optional for the user (observation/adoption remains the primary path) and SHALL respect the four-slot cap.

#### Scenario: Add-agent-pane creates and tracks a new pane
- **WHEN** the user triggers the add-agent-pane action on a session with fewer than four tracked panes
- **THEN** the system SHALL create a new tmux pane in that session running an agent
- **AND** the new pane SHALL be eligible for adoption into a slot

#### Scenario: Add-agent-pane blocked at the cap
- **WHEN** the user triggers the add-agent-pane action on a session that already tracks four panes
- **THEN** the system SHALL NOT create a fifth agent pane
- **AND** SHALL surface that the four-slot cap is reached
