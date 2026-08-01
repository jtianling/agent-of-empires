# multi-agent-session Specification

## Purpose
TBD - created by archiving change agent-session-recording. Update Purpose after archive.
## Requirements
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

### Requirement: A tracked pane relaunches as the agent its slot recorded

When restarting or recovering a tracked pane, the system SHALL build its command from the agent recorded on that pane's durable slot.

Instance-level launch concepts -- the command override, the pre-allocated conversation id, a pending fork token, extra arguments, and the instance's xats identity key -- describe the instance's one primary agent. They SHALL be applied only to slot 0, and only when that slot's recorded agent is the instance's tool. Both conditions are required: applying them to a slot 0 recording a different agent produces the wrong agent, and applying them to every slot recording the instance's tool would hand a single conversation id and identity to more than one pane.

#### Scenario: Adopted primary pane relaunches as its own agent
- **WHEN** an instance whose tool is a shell has slot 0 recording a different agent
- **AND** that slot is restarted or recovered
- **THEN** the pane SHALL be launched as the agent the slot recorded
- **AND** it SHALL NOT be launched as the instance's tool

#### Scenario: Instance-level launch context still applies to slot 0 running the instance's own agent
- **WHEN** slot 0 records the same agent as the instance's tool
- **THEN** the pane SHALL be launched with the instance's command override, pre-allocated conversation id, pending fork token, and extra arguments as before

#### Scenario: A later slot running the same agent stays secondary
- **WHEN** a slot other than slot 0 records the same agent as the instance's tool
- **THEN** it SHALL be built from that agent's binary
- **AND** it SHALL NOT receive the instance's pre-allocated conversation id, pending fork token, or xats identity key

#### Scenario: A mismatched slot does not receive the instance's conversation identity
- **WHEN** a slot records an agent that is not the instance's tool
- **THEN** the launched command SHALL NOT carry the instance's pre-allocated conversation id or pending fork token

