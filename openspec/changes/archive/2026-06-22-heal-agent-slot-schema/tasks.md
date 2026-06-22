## 1. Implement the self-heal

- [x] 1.1 In `ensure_schema` (`src/db/mod.rs`), after the `CREATE TABLE IF NOT EXISTS` batch, add an idempotent backfill: check via `pragma_table_info('agent_slot')` whether `tmux_pane` exists, and if not run `ALTER TABLE agent_slot ADD COLUMN tmux_pane TEXT NOT NULL DEFAULT ''`.
- [x] 1.2 Keep `upsert_agent_slot`, the reconciler, and the other tables (`pane_live`, `events`) unchanged.

## 2. Test (cover the fresh-db blind spot)

- [x] 2.1 Add a test that creates a legacy `agent_slot` (6 columns, no `tmux_pane`), seeds a row, then runs `ensure_schema` and asserts the `tmux_pane` column now exists and the existing row is preserved.
- [x] 2.2 Assert that after backfill `upsert_agent_slot` succeeds and the row is readable (no `no such column: tmux_pane`).
- [x] 2.3 Assert idempotency: running `ensure_schema` again, and against a fresh database that already has the column, succeeds with no duplicate-column error.

## 3. Verify

- [x] 3.1 `cargo fmt`, `cargo clippy` (no new warnings), `cargo test` green.
- [x] 3.2 aoe-tester e2e acceptance in isolated HOME (`~/workspace/test`): construct a legacy-schema db, exercise the real `aoe` capture+reconcile path, assert `agent_slot` populates (it could not before the fix); clean up self-opened tmux sessions. (PASS -- added tests/e2e/legacy_schema_heal.rs, 2 cases; 29/29 incl. regression green; sqlite3 teeth-check confirms RED without fix; zero leak.)
