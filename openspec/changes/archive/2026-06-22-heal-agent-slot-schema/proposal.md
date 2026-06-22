## Why

On databases created before the `agent_slot.tmux_pane` column was added to the schema DDL, the `agent_slot` table has only 6 columns. The schema is applied with `CREATE TABLE IF NOT EXISTS` (`src/db/mod.rs::ensure_schema`), which is a no-op for an already-existing table, so it never adds the column. `upsert_agent_slot` writes 7 columns (including `tmux_pane`), so on those legacy databases every upsert fails with `table agent_slot has no column named tmux_pane`. The error is swallowed by the reconciler's `Result` handling, so `agent_slot` stays permanently empty -- starving the `R` all-pane resume (w03) and cold-start recovery (w04) of their only data source. The migration version is already at the latest (`CURRENT_VERSION = 6`), so no migration repairs it, and the existing e2e suite uses fresh databases (which have the column), so the bug was fully masked.

## What Changes

- `ensure_schema` self-heals: after the `CREATE TABLE IF NOT EXISTS` batch, it checks whether `agent_slot` has the `tmux_pane` column and, if missing, runs `ALTER TABLE agent_slot ADD COLUMN tmux_pane TEXT NOT NULL DEFAULT ''`. Idempotent (skips when the column already exists).
- Because the store is profile-scoped and lazily created, every database (active and lazy) self-heals the next time it is opened via `open_with_schema` -- more robust than a migration that only touches the active profile.
- No data is lost (column added, table not recreated). No change to `upsert_agent_slot`, the reconciler, or the capture path.
- Not a breaking change. Closes a real data bug that prevented `agent_slot` from ever populating on legacy databases.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities
- `agent-session-store`: the store's schema application MUST backfill columns that were added to a table's DDL after the table was first created (specifically `agent_slot.tmux_pane`), so durable writes succeed on databases created by earlier versions, not only on fresh databases.

## Impact

- `src/db/mod.rs`: `ensure_schema` gains an idempotent column-backfill step for `agent_slot.tmux_pane`.
- Test coverage: add a test that opens a legacy-schema `agent_slot` (6 columns, no `tmux_pane`) and asserts the column is backfilled and `upsert_agent_slot` then succeeds -- covering the fresh-db blind spot in the current e2e suite.
- No migration, no data loss, no change to capture/reconcile logic.
