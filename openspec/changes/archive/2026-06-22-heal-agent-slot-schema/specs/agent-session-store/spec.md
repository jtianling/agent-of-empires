## ADDED Requirements

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
