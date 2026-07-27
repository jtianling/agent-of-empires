## MODIFIED Requirements

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

## ADDED Requirements

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
