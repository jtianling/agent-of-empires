## Context

The store schema is applied by `ensure_schema` (`src/db/mod.rs:249`) using a `CREATE TABLE IF NOT EXISTS` batch, called from `open_with_schema` (every store open) and `create_schema_for_profile` (the v006 migration). The `agent_slot` DDL gained a `tmux_pane TEXT NOT NULL DEFAULT ''` column after the table already shipped, but `CREATE TABLE IF NOT EXISTS` does not alter an existing table. Databases created before that column was added therefore keep a 6-column `agent_slot`. `upsert_agent_slot` writes 7 columns (including `tmux_pane`), so on those legacy databases every upsert fails (`table agent_slot has no column named tmux_pane`); the reconciler swallows the `Result`, so `agent_slot` never populates. Confirmed on a real db: the manual upsert errors and `agent_slot` is empty despite `pane_live` holding live captures.

## Goals / Non-Goals

**Goals:**
- Legacy databases self-heal: `agent_slot` gains the `tmux_pane` column on the next store open, without data loss.
- Idempotent: re-running is safe; fresh databases (which already have the column) are unaffected.
- No change to `upsert_agent_slot`, the reconciler, or the capture path.

**Non-Goals:**
- Recreating or migrating table data (column add only).
- A general schema-diff / all-columns reconciler. Only the one known missing column (`agent_slot.tmux_pane`) is backfilled; broader drift is out of scope (YAGNI).
- Bumping the migration version. The fix lives in `ensure_schema` so it covers all profile databases (active and lazily created), which a single-active-profile migration would not.

## Decisions

### Decision 1: Self-heal in `ensure_schema`, not a new migration
Add the backfill to `ensure_schema`, after the `CREATE TABLE IF NOT EXISTS` batch.

Rationale:
- The store is profile-scoped and lazily created (`open_with_schema` runs on first touch per profile). `ensure_schema` runs on every open, so the backfill reaches every database -- the active profile and every lazily-opened one.
- A v007 migration would only run `create_schema_for_profile` for the active profile (matching v006's scope), leaving other profiles' legacy databases unhealed until separately touched. `ensure_schema` is the single DDL source of truth and is already documented as idempotent.

### Decision 2: Check-then-ALTER via `pragma_table_info`
Detect the column with `pragma_table_info('agent_slot')` and only `ALTER TABLE agent_slot ADD COLUMN tmux_pane TEXT NOT NULL DEFAULT ''` when absent.

Rationale: SQLite `ADD COLUMN` errors on a duplicate column, so blindly running it and ignoring errors would mask unrelated failures. An explicit existence check keeps the operation cleanly idempotent. `ADD COLUMN ... NOT NULL DEFAULT ''` is allowed by SQLite because a constant default is supplied; existing rows (none on the real db) get `''`.

### Decision 3: Scope the backfill to the known column only
Only `agent_slot.tmux_pane` is backfilled. Other tables (`pane_live`, `events`) match their DDL on the real db and have no known drift. A generic column reconciler is deferred until a second case appears.

## Risks / Trade-offs

- **A different table/column drifts in the future** -> not covered here; revisit with a generic approach if a second case appears. Logged as a Non-Goal rather than pre-built.
- **Extra `pragma_table_info` query on every store open** -> negligible (one cheap pragma on a tiny table), bounded by how often the store is opened.
- **`ADD COLUMN` on a large `agent_slot`** -> not a concern; capped at 4 rows per instance and the real table is empty.

## Migration Plan

No migration version bump and no data migration -- the backfill is part of the idempotent `ensure_schema` and runs on the next store open for each profile. Rollback is a straight revert; an already-backfilled `tmux_pane` column is harmless (it matches the current DDL for fresh databases).
