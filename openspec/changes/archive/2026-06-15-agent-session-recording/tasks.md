## 1. SQLite store foundation

- [x] 1.1 Add `rusqlite` (with `bundled` feature) to `Cargo.toml`
- [x] 1.2 Create the store module (e.g. `src/db/mod.rs` or `src/session/store.rs`) that opens `aoe.db` in the active profile dir, enables WAL mode and a short busy-timeout, and exposes a connection/handle
- [x] 1.3 Add migration `src/migrations/vNNN_agent_session_store.rs` creating `pane_live`, `agent_slot`, and `events` tables with the keys/constraints from the spec (idempotent `CREATE TABLE IF NOT EXISTS`); register it and bump `CURRENT_VERSION` in `src/migrations/mod.rs`
- [x] 1.4 Implement store API: `upsert_pane_live`, `delete_pane_live`, `upsert_agent_slot`, `delete_slots_for_instance`, `append_event`, and read helpers; enforce the slot 0..3 range
- [x] 1.5 Unit tests for the store using a temp-dir DB (upsert-by-key, slot range rejection, append-only events, profile isolation, idempotent migration)

## 2. Pane session capture (hook)

- [x] 2.1 Add a hidden `aoe __record-pane` subcommand in `src/cli/` that reads hook stdin JSON, extracts `session_id`/`cwd`, reads `$TMUX_PANE` and agent from env/args, and performs a `pane_live` upsert; it MUST exit 0 on any error and never block
- [x] 2.2 Rewrite the installed status hook command in `src/hooks/mod.rs`: keep the existing status-file write; add the `$TMUX_PANE`-gated branch that invokes the absolute path of the running `aoe` binary with `__record-pane`; remove the dead `$CLAUDE_SESSION_ID`/`$CODEX_SESSION_ID` env capture line
- [x] 2.3 Resolve and bake the absolute `aoe` binary path into the hook command at install time so it works regardless of `$PATH`
- [x] 2.4 Unit tests for hook command construction (TMUX_PANE branch present, stdin-based, no env-var capture line, absolute binary path)

## 3. Reconciler

- [x] 3.1 Implement the reconcile routine: per managed session, `tmux list-panes`, resolve each pane via `pane_live`, assign slots deterministically (primary `@aoe_agent_pane` = slot 0; others by ascending pane index), upsert `agent_slot`, cap at 4
- [x] 3.2 Garbage-collect orphan `pane_live` rows whose `tmux_pane` is not in any managed session
- [x] 3.3 Hook the reconcile routine into the existing `src/tui/status_poller.rs` tick
- [x] 3.4 Append `capture`/`adopt` events when a new slot is first recorded for a session
- [x] 3.5 Unit tests for slot assignment (primary pinned to slot 0, ordering, 4-cap) and orphan GC

## 4. Multi-agent session model

- [x] 4.1 Add adoption support: a managed session begins tracking an agent that appears in any of its panes (observe-first), recording it via the reconciler; enforce the 4-slot cap
- [x] 4.2 Add the optional "add agent pane" action (split the tmux window + launch an agent), respecting the 4-slot cap and surfacing when the cap is reached; wire a TUI key or CLI subcommand without colliding with existing bindings
- [x] 4.3 Purge a session's `agent_slot`/`pane_live` rows on session deletion (wire into the existing delete path)
- [x] 4.4 Unit/integration tests for adoption, the 4-cap, and delete cleanup

## 5. Verification and docs

- [x] 5.1 Run `cargo fmt`, `cargo clippy`, and `cargo test` clean
- [x] 5.2 E2E: 19 full-binary tests (real `aoe` + tmux, isolated `$HOME`) across `tests/e2e/{agent_session_store,pane_session_capture,multi_agent_session}.rs` assert store creation/migration, `$TMUX_PANE`-keyed capture (incl. hand-launched without `$AOE_INSTANCE_ID`), reconciler snapshot + sticky 4-cap + orphan GC, multi-pane adoption, and durable records surviving restart -- all green. Real-`claude` hook firing (`$TMUX_PANE` + stdin `session_id`) was proven by an earlier live spike and an independent hook-snippet replay; driving a full real-`claude` process against the real `~/.claude` is intentionally not automated (it would rewrite the user's live `~/.claude/settings.json` to the worktree binary).
- [x] 5.3 Update any user-facing docs in `docs/` if the add-agent-pane action introduces a new keybinding/command (and re-run `cargo xtask gen-docs` if CLI help changed)
