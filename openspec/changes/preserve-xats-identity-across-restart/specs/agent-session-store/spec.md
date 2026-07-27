## MODIFIED Requirements

### Requirement: Durable per-slot agent record
The store SHALL provide an `agent_slot` table holding the durable mapping: `instance_id` (text), `slot` (integer, 0..3), `agent` (text), `native_session_id` (text), `cwd` (text), `tmux_pane` (text, the pane currently mapped to the slot), `xats_identity_key` (text, the opaque xats identity key minted for the slot, empty when the slot has none), `last_seen_at` (timestamp), with a primary key of `(instance_id, slot)`. The `slot` value SHALL be constrained to the range 0 through 3 (at most 4 slots per instance). The `tmux_pane` value lets the reconciler keep an already-tracked pane in its slot (sticky assignment).

#### Scenario: Upsert by instance and slot
- **WHEN** a snapshot writes `(instance_id, slot)` that already exists
- **THEN** the existing row SHALL be updated in place (no duplicate row for the same key)

#### Scenario: Slot range enforced
- **WHEN** a write attempts a `slot` value outside 0..3
- **THEN** the store SHALL reject the write

#### Scenario: Records survive process restart
- **WHEN** an `agent_slot` row is written and AoE is closed and reopened
- **THEN** the row SHALL be readable from `aoe.db` after restart with the same `native_session_id`

#### Scenario: Identity key survives process restart
- **WHEN** an `agent_slot` row carrying an `xats_identity_key` is written and AoE is closed and reopened
- **THEN** the row SHALL be readable with the same `xats_identity_key`

## ADDED Requirements

### Requirement: Reconcile preserves a slot's identity key
The reconciler assigns panes to slots and rewrites durable slot records from pane captures. A capture carries no identity key, so reconcile SHALL preserve the key already recorded on the slot rather than overwriting it with an empty value.

#### Scenario: Capture does not clear the slot's key
- **WHEN** a durable slot record carrying an identity key is rewritten from a new pane capture
- **THEN** the record SHALL retain its identity key

#### Scenario: Slot with no key stays empty
- **WHEN** a durable slot record with no identity key is rewritten from a pane capture
- **THEN** the record's identity key SHALL remain empty

### Requirement: Schema healing covers the identity key column
Databases created before the identity key column existed SHALL be healed by the existing idempotent schema path rather than by ad-hoc fallback logic in the main code path, matching how the durable slot pane column is healed.

#### Scenario: Legacy database gains the column
- **WHEN** the store opens a database whose `agent_slot` table predates the `xats_identity_key` column
- **THEN** the schema path SHALL add the column
- **AND** subsequent slot writes SHALL succeed

#### Scenario: Healing is idempotent
- **WHEN** the schema path runs against a database that already has the column
- **THEN** it SHALL leave the table unchanged
