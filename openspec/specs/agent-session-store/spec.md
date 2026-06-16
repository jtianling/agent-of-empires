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
The store SHALL provide an `agent_slot` table holding the durable mapping: `instance_id` (text), `slot` (integer, 0..3), `agent` (text), `native_session_id` (text), `cwd` (text), `tmux_pane` (text, the pane currently mapped to the slot), `last_seen_at` (timestamp), with a primary key of `(instance_id, slot)`. The `slot` value SHALL be constrained to the range 0 through 3 (at most 4 slots per instance). The `tmux_pane` value lets the reconciler keep an already-tracked pane in its slot (sticky assignment).

#### Scenario: Upsert by instance and slot
- **WHEN** a snapshot writes `(instance_id, slot)` that already exists
- **THEN** the existing row SHALL be updated in place (no duplicate row for the same key)

#### Scenario: Slot range enforced
- **WHEN** a write attempts a `slot` value outside 0..3
- **THEN** the store SHALL reject the write

#### Scenario: Records survive process restart
- **WHEN** an `agent_slot` row is written and AoE is closed and reopened
- **THEN** the row SHALL be readable from `aoe.db` after restart with the same `native_session_id`

### Requirement: Append-only event stream
The store SHALL provide an `events` table recording status and lifecycle events: `id` (autoincrement), `instance_id` (text), `slot` (integer, nullable), `kind` (text, e.g. `status`, `capture`, `adopt`), `detail` (text, nullable), `created_at` (timestamp). Event rows SHALL be append-only.

#### Scenario: Event appended
- **WHEN** the system records an event for an instance
- **THEN** a new row SHALL be inserted into `events` with a monotonically increasing `id`
- **AND** existing event rows SHALL NOT be modified

### Requirement: Store cleanup on session deletion
When a session is deleted, the system SHALL remove that session's `agent_slot` rows and any `pane_live` rows whose `tmux_pane` belonged to that session.

#### Scenario: Deleting a session purges its durable records
- **WHEN** a session with `instance_id = X` is deleted
- **THEN** all `agent_slot` rows with `instance_id = X` SHALL be removed
- **AND** event rows MAY be retained for history

