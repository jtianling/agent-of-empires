## ADDED Requirements

### Requirement: An extra pane AoE launches has a durable slot record from launch

When AoE launches an extra agent pane into a session, it SHALL write that pane's durable slot record at launch time rather than waiting for the pane's first capture. The launch-time record SHALL carry the agent, the pane id AoE just created, and the identity key it minted, and SHALL carry no native session id, because the pane has not yet reported a conversation.

AoE SHALL write the primary pane's record at that same moment when it has none, because the restart fan-out reads only the slots that exist and would otherwise reach the extra pane while skipping the pane beside it. That record carries no identity key: the primary pane's key lives on the instance record.

A session that launches a single pane is unchanged: its slot record still arrives with its first capture. The launch-time write is what an extra pane needs to be tracked at all, not a general replacement for capture-driven adoption.

Capture is observe-first and can lag arbitrarily: a Codex pane is claimed only once its rollout file lands, which happens after its first exchange. A slot record that exists only after that point leaves the pane untracked and unrestartable in the meantime, and leaves a launched key nowhere to live.

A record with no native session id SHALL be a valid state, not an error.

#### Scenario: The slot record exists before the pane's first capture

- **WHEN** AoE launches an extra agent pane into a slot
- **THEN** a durable slot record for that pane SHALL exist immediately, carrying the pane id and the launched agent
- **AND** the record's native session id SHALL be empty until a capture supplies one

#### Scenario: The primary pane beside it is tracked too

- **WHEN** AoE launches an extra agent pane into a session whose primary pane has no durable slot record
- **THEN** a launch-time record SHALL be written for the primary pane as well, so a restart reaches both panes
- **AND** an existing primary record SHALL be left alone, because it carries a captured conversation a launch-time record would blank

#### Scenario: A launch write never carries a conversation over

- **WHEN** a launch-time record is written for a slot that already records a conversation, including one recorded against the same pane id
- **THEN** the slot SHALL be left with no conversation
- **AND** a capture that landed in that window SHALL be restored by the next reconcile from the volatile capture it never touched

#### Scenario: A capture write keeps a key stored after its caller read it

- **WHEN** a capture is snapshotted into a slot whose stored identity key changed after the caller read it
- **THEN** the stored key SHALL survive the snapshot
- **AND** the key the caller carried SHALL apply only to a slot that has none

#### Scenario: Reclaiming a slot drops the previous pane's conversation and key

- **WHEN** a launch-time record is written for a slot whose existing record names a different pane
- **THEN** the slot SHALL carry the new pane's identity key and no conversation
- **AND** the previous pane's conversation and key SHALL NOT be inherited

#### Scenario: A capture completes the record without replacing its key

- **WHEN** a capture arrives for a pane whose slot record was written at launch
- **THEN** the reconciler SHALL fill in the native session id from that capture
- **AND** the identity key already on the record SHALL be preserved

#### Scenario: The launch-time record keeps its slot

- **WHEN** the reconciler runs while a launch-time slot record's pane is still live
- **THEN** that pane SHALL stay in the slot its launch-time record named

#### Scenario: A record with no native session id does not fail a launch

- **WHEN** a pane is launched from a slot record that carries no native session id
- **THEN** the launch SHALL proceed without a resume token rather than reporting an error
