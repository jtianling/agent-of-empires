# agent-session-store Specification

## Purpose
TBD - created by archiving change agent-session-recording. Update Purpose after archive.
## Requirements
### Requirement: SQLite store is created under the active profile directory
The system SHALL maintain a SQLite database named `aoe.db` in the active profile directory (alongside `sessions.json`). The database SHALL be created and have its schema applied through the existing `src/migrations/` system, not by ad-hoc code in the main path.

#### Scenario: Database created on first run
- **WHEN** AoE starts and `aoe.db` does not yet exist in the profile directory
- **THEN** the migration system SHALL create `aoe.db`
- **AND** apply the schema (all required tables) before any store read or write

#### Scenario: Migration is idempotent
- **WHEN** the schema migration runs and `aoe.db` already has the current schema
- **THEN** the migration SHALL complete without error
- **AND** SHALL NOT duplicate or drop existing rows

#### Scenario: Store path is profile-scoped
- **WHEN** two different profiles are active in turn
- **THEN** each profile SHALL use its own `aoe.db` under that profile's directory
- **AND** records SHALL NOT leak between profiles

### Requirement: Volatile per-pane capture table
The store SHALL provide a `pane_live` table holding the latest capture per tmux pane: `tmux_pane` (text, primary key), `agent` (text), `native_session_id` (text), `cwd` (text), `updated_at` (timestamp). Writes SHALL upsert by `tmux_pane`.

#### Scenario: Upsert by tmux pane
- **WHEN** two captures arrive for the same `tmux_pane` with different `native_session_id`
- **THEN** the row for that `tmux_pane` SHALL reflect the most recent capture
- **AND** there SHALL be exactly one row for that `tmux_pane`

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

### Requirement: Append-only event stream
The store SHALL provide an `events` table recording status and lifecycle events: `id` (autoincrement), `instance_id` (text), `slot` (integer, nullable), `kind` (text, e.g. `status`, `capture`, `adopt`), `detail` (text, nullable), `created_at` (timestamp). Event rows SHALL NOT be modified once written.

The stream SHALL be bounded. On schema application the store SHALL drop event rows older than a retention window, and SHALL keep at most a fixed number of the most recent rows per instance, so neither an old quiet database nor one busy instance can grow the table without limit. When a prune removes rows, the store SHALL reclaim the freed space so an already-oversized database shrinks on disk.

#### Scenario: Event appended
- **WHEN** the system records an event for an instance
- **THEN** a new row SHALL be inserted into `events` with a monotonically increasing `id`
- **AND** existing event rows SHALL NOT be modified

#### Scenario: Events older than the retention window are dropped
- **WHEN** the schema is applied to a store holding event rows older than the retention window
- **THEN** those rows SHALL be removed
- **AND** rows inside the window SHALL be retained

#### Scenario: Per-instance row cap is enforced
- **WHEN** an instance has more recent event rows than the per-instance cap
- **AND** the schema is applied
- **THEN** only the most recent rows up to the cap SHALL be retained for that instance
- **AND** another instance's rows SHALL NOT be removed to make room

#### Scenario: Pruning reclaims space
- **WHEN** a prune removes event rows
- **THEN** the store SHALL reclaim the freed space rather than leaving the file at its previous size

#### Scenario: A store within its bounds is left alone
- **WHEN** the schema is applied to a store whose events are inside the retention window and under the cap
- **THEN** no event rows SHALL be removed
- **AND** no space reclamation SHALL be performed

### Requirement: Store cleanup on session deletion
When a session is deleted, the system SHALL remove that session's `agent_slot` rows, its layout snapshot, its event rows, and any `pane_live` rows whose `tmux_pane` belonged to that session.

#### Scenario: Deleting a session purges its durable records
- **WHEN** a session with `instance_id = X` is deleted
- **THEN** all `agent_slot` rows with `instance_id = X` SHALL be removed
- **AND** all `events` rows with `instance_id = X` SHALL be removed

#### Scenario: Another session's records survive
- **WHEN** a session with `instance_id = X` is deleted
- **THEN** rows belonging to other instances SHALL be left intact

### Requirement: Schema heals columns missing from legacy databases
The store's schema application SHALL be safe to run against databases created by earlier versions whose tables predate later-added columns. Because tables are created with `CREATE TABLE IF NOT EXISTS` (which does not alter an existing table), the schema application SHALL, after creating the tables, ensure that `agent_slot` has the `tmux_pane` column and add it (`ALTER TABLE agent_slot ADD COLUMN tmux_pane TEXT NOT NULL DEFAULT ''`) when it is absent. This backfill SHALL be idempotent (a no-op when the column already exists), SHALL NOT recreate the table or lose existing rows, and SHALL run on every store open so that every profile's database (active and lazily created) self-heals.

#### Scenario: Legacy agent_slot gains the missing column on open
- **WHEN** a database has an `agent_slot` table without the `tmux_pane` column (created by an earlier version)
- **AND** the store schema is applied (store opened)
- **THEN** the `agent_slot` table SHALL afterward have a `tmux_pane` column
- **AND** existing `agent_slot` rows SHALL be preserved (column added, table not recreated)

#### Scenario: Durable upsert succeeds after backfill
- **WHEN** a legacy database has been opened and its `agent_slot` column backfilled
- **AND** the reconciler upserts an `agent_slot` record (with a `tmux_pane` value)
- **THEN** the upsert SHALL succeed and the row SHALL be readable
- **AND** the reconciler SHALL no longer fail with `no such column: tmux_pane`

#### Scenario: Backfill is idempotent
- **WHEN** the store schema is applied to a database whose `agent_slot` already has the `tmux_pane` column (a fresh database, or one already healed)
- **THEN** the schema application SHALL succeed without error
- **AND** SHALL NOT attempt to add a duplicate column

### Requirement: An unreadable store is quarantined, not fatal
When the schema cannot be applied because the database is corrupt or is not a database, the store SHALL move the file aside under a timestamped name and create an empty database in its place, so the profile remains usable.

The store SHALL NOT attempt to repair or salvage the quarantined file, and SHALL NOT delete it. Failures that are not corruption (permissions, locking, a missing directory) SHALL continue to surface as ordinary errors.

#### Scenario: Corrupt database is moved aside and recreated
- **WHEN** the store is opened with schema application against a corrupt database file
- **THEN** the corrupt file SHALL be preserved under a timestamped name
- **AND** a new empty database SHALL be created in its place
- **AND** the open SHALL succeed

#### Scenario: A file that is not a database is quarantined the same way
- **WHEN** the store is opened with schema application against a file that is not a SQLite database
- **THEN** the file SHALL be preserved under a timestamped name
- **AND** the open SHALL succeed against a new empty database

#### Scenario: Startup survives a corrupt store
- **WHEN** AoE starts in a profile whose database is corrupt
- **THEN** it SHALL start
- **AND** it SHALL NOT abort with a database error

#### Scenario: Non-corruption failures are not quarantined
- **WHEN** the store cannot be opened for a reason other than corruption
- **THEN** the file SHALL NOT be moved aside
- **AND** the error SHALL be returned to the caller

### Requirement: Quarantine is surfaced to the user
When a database is quarantined, the system SHALL warn the user, naming the path the unreadable file was preserved at, rather than recovering silently.

#### Scenario: User is told where the quarantined file went
- **WHEN** a profile's database is quarantined during startup
- **THEN** the user SHALL be shown a warning identifying the preserved file's path

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

### Requirement: An extra pane AoE launches has a durable slot record from launch

When AoE launches an extra agent pane into a session, it SHALL write that pane's durable slot record at launch time rather than waiting for the pane's first capture. The launch-time record SHALL carry the agent, the pane id AoE just created, the working directory the pane was launched into, and the identity key it minted, and SHALL carry no native session id, because the pane has not yet reported a conversation.

The working directory on that record SHALL be the launched pane's own, not the instance's. The two are equal only when the pane was launched into the session's directory. Recording the instance's directory for a pane launched elsewhere produces a record that is correct at launch and wrong at the first restart, because restart and cold-start recovery both place a pane at the directory its slot recorded.

AoE SHALL write the primary pane's record at that same moment when it has none, because the restart fan-out reads only the slots that exist and would otherwise reach the extra pane while skipping the pane beside it. That record carries the instance's working directory and no identity key: the primary pane's key lives on the instance record.

A session that launches a single pane is unchanged: its slot record still arrives with its first capture. The launch-time write is what an extra pane needs to be tracked at all, not a general replacement for capture-driven adoption.

A shell pane runs no agent, holds no identity and produces no capture, so it SHALL stay slotless when it was launched into the session's own directory: a slot would cost the session one of four and recovery would place the pane there regardless. A shell pane launched into any other directory SHALL be recorded. That directory is held nowhere else, so without a record the pane is outside the restart fan-out and absent from cold recovery entirely, and the directory the user chose would survive only until the first restart.

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

#### Scenario: A shell pane in the session's directory stays slotless

- **WHEN** AoE launches a shell pane into the session's own working directory
- **THEN** no slot record SHALL be written for it

#### Scenario: A shell pane with a directory of its own is recorded

- **WHEN** AoE launches a shell pane into a directory other than the session's
- **THEN** a slot record SHALL be written for it carrying that directory
- **AND** a restart SHALL relaunch it as the user's shell in that directory

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

