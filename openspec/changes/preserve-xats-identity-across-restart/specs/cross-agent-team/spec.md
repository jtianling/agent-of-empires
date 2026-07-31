## ADDED Requirements

### Requirement: Cross Agent Team panes carry a durable identity key

When Cross Agent Team is enabled for a pane, AoE SHALL associate that pane with an opaque identity key and SHALL inject it into the launched pane as the `XATS_IDENTITY_KEY` environment variable. The key SHALL be minted by AoE, SHALL be treated as an opaque value that AoE never interprets, and SHALL be injected on every launch of that pane regardless of restart mode, including the pane's first launch.

AoE SHALL NOT read, store, display, or configure a xats team or agent name.

#### Scenario: Key injected on first launch

- **WHEN** a Cross Agent Team pane is launched for the first time
- **THEN** AoE SHALL mint an identity key for it
- **AND** the launched pane's environment SHALL contain that key as `XATS_IDENTITY_KEY`

#### Scenario: Key injected for both supported tools

- **WHEN** a Cross Agent Team pane is launched for `claude` or for `codex`
- **THEN** the launched pane's environment SHALL contain the pane's identity key

#### Scenario: Key is distinct from the codex pane pre-registration nonce

- **WHEN** a Cross Agent Team `codex` pane is launched
- **THEN** the pane SHALL carry both its durable identity key and a freshly generated single-use pane pre-registration nonce
- **AND** the two values SHALL be different

#### Scenario: No key when the feature is disabled

- **WHEN** a session is launched with Cross Agent Team disabled
- **THEN** AoE SHALL NOT mint or inject an identity key

#### Scenario: Key is not exposed through command arguments

- **WHEN** a Cross Agent Team pane is launched
- **THEN** the identity key SHALL NOT appear in the launch command's arguments
- **AND** it SHALL NOT be written to logs

### Requirement: Identity key storage follows the pane's role

The primary pane's identity key SHALL be stored on the instance record, alongside the other state describing that same agent (its pre-allocated session id, resume token, and pending fork). An adopted pane's identity key SHALL be stored on its durable slot record.

#### Scenario: Primary key survives with the instance record

- **WHEN** a Cross Agent Team session's primary pane has an identity key and AoE is closed and reopened
- **THEN** the instance record SHALL still carry that key

#### Scenario: Adopted slot key survives with the slot record

- **WHEN** an adopted pane's slot has an identity key and AoE is closed and reopened
- **THEN** the durable slot record SHALL still carry that key

### Requirement: Panes AoE never launched receive a key at their first relaunch

Agent panes are adopted observe-first: a user may split a pane and start an agent in it by hand, and AoE never builds that pane's launch command. AoE SHALL NOT attempt to inject a key into such a pane while it is running. It SHALL mint and inject one the first time it launches that pane's slot itself, after which the key is stable like any other.

The consequence is bounded rather than permanent: the key is bound to the identity during the registration that follows its first injection, so such a pane costs one extra manual registration and recovers normally from then on.

#### Scenario: Hand-started pane has no key until AoE relaunches it

- **WHEN** a user starts an agent by hand in a split pane of a Cross Agent Team session
- **AND** the reconciler adopts that pane into a slot
- **THEN** the slot SHALL carry no identity key
- **AND** AoE SHALL NOT alter the running pane

#### Scenario: First AoE relaunch mints the slot's key

- **WHEN** AoE launches an adopted slot that has no identity key
- **THEN** AoE SHALL mint one, persist it on the slot, and inject it into the launched pane
- **AND** subsequent launches of that slot SHALL reuse it

#### Scenario: Key that is not yet bound does not fail the launch

- **WHEN** a pane is launched with a freshly minted identity key that no identity has been registered against yet
- **THEN** AoE SHALL treat the launch as successful
- **AND** SHALL retain the key so the registration that follows can bind it

### Requirement: Identity key is stable across relaunch, restart, and recovery

A pane's identity key SHALL be minted once and reused on every subsequent launch of that pane's slot. Resume restart, clean restart, resume recovery, and clean recovery SHALL all inject the slot's existing key rather than minting a new one.

#### Scenario: Clean restart reuses the key

- **WHEN** a Cross Agent Team session is restarted clean
- **THEN** each relaunched pane's environment SHALL contain the same identity key it carried before the restart

#### Scenario: Clean recovery reuses the key

- **WHEN** a recoverable Cross Agent Team instance is recovered in fresh mode
- **THEN** each recovered pane SHALL be launched with the identity key stored on its durable slot record

#### Scenario: Key survives AoE restart

- **WHEN** an identity key has been persisted for a slot and AoE is closed and reopened
- **THEN** the same key SHALL be injected on the next launch of that slot

#### Scenario: The launch that mints the key persists it

- **WHEN** a Cross Agent Team session is launched and that launch mints the instance's identity key
- **THEN** the minted key SHALL be stored on the session record as part of that launch
- **AND** the next restart SHALL inject the stored key rather than minting a new one

Minting the key on a working copy of the instance and discarding it leaves the record keyless, so the first restart mints a second key. The daemon then finds no holder for the new key and treats the restarted pane as a new identity instead of a recovering one, while the old key stays bound to the dead pane.

### Requirement: Cloned and forked sessions receive a fresh identity key

When a session is created from an existing session through new-from-selection, or when a pane is forked, AoE SHALL mint a new identity key for the resulting pane and SHALL NOT copy the source pane's key.

This is the only point at which two panes claiming one identity can be prevented. Once a copied key has been bound, the daemon cannot distinguish a pane recovering its own identity from a pane presenting a copied key.

#### Scenario: New-from-selection does not inherit the key

- **WHEN** a Cross Agent Team session is created from an existing session through new-from-selection
- **THEN** the new session's pane SHALL carry an identity key different from the source pane's key

#### Scenario: Fork does not inherit the key

- **WHEN** a Cross Agent Team pane is forked
- **THEN** the forked pane SHALL carry an identity key different from its parent's key

### Requirement: Unresolvable identity key degrades to normal registration

An identity key that no longer corresponds to a known identity SHALL be treated as a normal state. AoE SHALL NOT report an error, SHALL NOT clear the stored key, and SHALL leave the pane usable so the user can register it the same way they do without a key.

#### Scenario: Key no longer resolves

- **WHEN** a pane is launched with a stored identity key that no longer corresponds to a known identity
- **THEN** AoE SHALL NOT surface an error for the session
- **AND** AoE SHALL retain the stored key for future launches
- **AND** the pane SHALL remain usable for manual registration
