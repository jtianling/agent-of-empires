## MODIFIED Requirements

### Requirement: An extra pane AoE launches has a durable slot record from launch

When AoE launches an extra managed pane into a session, it SHALL write that pane's durable slot record at launch time rather than waiting for the pane's first capture. The launch-time record SHALL carry the agent or shell tool, the pane id AoE just created, the working directory the pane was launched into, and any identity key it minted, and SHALL carry no native session id, because the pane has not yet reported a conversation.

The working directory on that record SHALL be the launched pane's own, not the instance's. The two are equal only when the pane was launched into the session's directory. Recording the instance's directory for a pane launched elsewhere produces a record that is correct at launch and wrong at the first restart, because restart and cold-start recovery both place a pane at the directory its slot recorded.

AoE SHALL write the primary pane's record at that same moment when it has none, because the restart fan-out reads only the slots that exist and would otherwise reach the extra pane while skipping the pane beside it. That record carries the instance's working directory and no identity key: the primary pane's key lives on the instance record.

A session that launches a single pane is unchanged: its slot record still arrives with its first capture. The launch-time write is what an extra pane needs to be tracked at all, not a general replacement for capture-driven adoption.

Every shell pane that AoE explicitly launches as a managed pane SHALL receive a durable slot, including one launched into the session's own directory. A shell pane runs no agent, holds no identity and produces no capture, so its launch-time record remains authoritative until the pane is relaunched or removed. The slot is required because restart and cold-start recovery fan out from durable slots, not from the transient tmux layout.

A shell slot SHALL be relaunched as the user's shell rather than through the agent registry's binary for `shell`, which names no program.

Capture is observe-first and can lag arbitrarily: a Codex pane is claimed only once its rollout file lands, which happens after its first exchange. A slot record that exists only after that point leaves the pane untracked and unrestartable in the meantime, and leaves a launched key nowhere to live.

A record with no native session id SHALL be a valid state, not an error.

#### Scenario: The slot record exists before the pane's first capture

- **WHEN** AoE launches an extra agent pane into a slot
- **THEN** a durable slot record for that pane SHALL exist immediately, carrying the pane id and the launched agent
- **AND** the record's native session id SHALL be empty until a capture supplies one

#### Scenario: The record carries the directory the pane was launched into

- **WHEN** AoE launches an extra agent pane into a directory other than the instance's
- **THEN** that pane's launch-time record SHALL carry the directory it was launched into

#### Scenario: A managed shell pane in the session's directory is recorded

- **WHEN** AoE launches a managed shell pane into the session's own working directory
- **THEN** a durable slot record SHALL be written for it carrying that directory
- **AND** a restart SHALL relaunch it as the user's shell in that directory

#### Scenario: A shell pane with a directory of its own is recorded

- **WHEN** AoE launches a shell pane into a directory other than the session's
- **THEN** a slot record SHALL be written for it carrying that directory
- **AND** a restart SHALL relaunch it as the user's shell in that directory

#### Scenario: Cold recovery includes a managed shell pane

- **WHEN** durable slots contain a primary agent pane and a managed shell pane
- **AND** the tmux session no longer exists
- **THEN** cold recovery SHALL recreate both panes from their slots
- **AND** each pane SHALL start in the directory carried by its own slot

#### Scenario: The primary pane beside it is tracked too

- **WHEN** AoE launches an extra agent pane into a session whose primary pane has no durable slot record
- **THEN** a launch-time record SHALL be written for the primary pane as well, so a restart reaches both panes
- **AND** that record SHALL carry the instance's working directory, not the extra pane's
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

#### Scenario: A capture corrects a recorded directory

- **WHEN** a pane whose slot records one working directory reports a capture from another
- **THEN** the slot SHALL be updated to the captured directory
- **AND** the slot's identity key SHALL be preserved
