## ADDED Requirements

### Requirement: Extra agent panes AoE launches carry an identity key from their first launch

When AoE launches an additional agent pane for a Cross Agent Team session, it SHALL mint an identity key for that pane at launch, persist it on the pane's durable slot record, and inject it into the launched process as `XATS_IDENTITY_KEY`. This covers both launch entry points: the right pane of a new session, and `aoe session add-agent-pane`.

The key SHALL be freshly minted for that pane. AoE SHALL NOT reuse the instance's own key for it: two live panes presenting one identity is the state the recovery design cannot resolve, and it is preventable only at the moment the second pane is launched.

An extra pane is not a pane AoE never launched. AoE builds its command and knows its slot, so the allowance that a pane may run keyless until its first relaunch applies only to panes a user started by hand.

The key travels the same route as the primary pane's: an environment assignment prefixing the pane's start command, which the pane's shell consumes before the agent runs. The agent process therefore never carries it in its arguments. It does remain readable from the pane's recorded start command by anything that can talk to the same tmux server, which is a property of the existing injection route rather than of this change, and is not addressed here.

#### Scenario: Right pane of a new session is launched with a key

- **WHEN** a Cross Agent Team session is created with a right pane agent tool
- **THEN** the right pane process environment SHALL contain `XATS_IDENTITY_KEY`
- **AND** the key SHALL be recorded on that pane's durable slot record

#### Scenario: A pane added through the CLI is launched with a key

- **WHEN** `aoe session add-agent-pane` launches an agent pane into a Cross Agent Team session
- **THEN** the launched pane's environment SHALL contain `XATS_IDENTITY_KEY`

#### Scenario: The extra pane's key is not the primary pane's key

- **WHEN** a Cross Agent Team session is created with a right pane agent tool
- **THEN** the right pane's identity key SHALL differ from the key injected into the primary pane

#### Scenario: The extra pane's key is reused rather than reminted on restart

- **WHEN** a session whose extra pane was launched with a key is restarted
- **THEN** the relaunched extra pane SHALL carry the same identity key it carried before the restart

#### Scenario: No key when Cross Agent Team is disabled

- **WHEN** an extra agent pane is launched for a session that does not have Cross Agent Team enabled
- **THEN** no identity key SHALL be minted and no `XATS_IDENTITY_KEY` SHALL be injected

#### Scenario: A shell extra pane receives no key

- **WHEN** an extra pane is launched with the `shell` tool
- **THEN** no identity key SHALL be minted for it

#### Scenario: Failing to record the key is reported, not swallowed

- **WHEN** an extra agent pane is launched but its identity key cannot be persisted
- **THEN** the failure SHALL be surfaced to the user rather than only logged
- **AND** the pane SHALL be left running, because it is usable and relaunching it would not repair the record

#### Scenario: Key reaches the agent through the environment, not its arguments

- **WHEN** an extra agent pane is launched with an identity key
- **THEN** the key SHALL be passed as an environment assignment that the pane's shell consumes
- **AND** the key value SHALL NOT appear in the arguments of the agent process itself
