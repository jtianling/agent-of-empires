## ADDED Requirements

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
