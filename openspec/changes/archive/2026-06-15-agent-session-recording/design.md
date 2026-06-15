## Context

AoE currently manages exactly one agent pane per session (`@aoe_agent_pane`). The only durable trace of an agent's identity is a per-instance `session_id` file written under `/tmp/aoe-hooks/<instance_id>/` by a status hook installed in the agent's `settings.json`. That mechanism has three problems for "manage every agent in a session, survive reboot, restart-and-resume":

1. The hook gates on `$AOE_INSTANCE_ID` (`[ -n "$AOE_INSTANCE_ID" ] || exit 0`), so an agent the user starts by hand in a shell pane is never recorded.
2. The capture is one `session_id` file per instance, so multiple agents in one session overwrite each other.
3. `/tmp` is wiped on reboot.

### Spike findings (Claude 2.1.177, verified)

A live spike ran a real `claude` inside a dedicated tmux pane with a SessionStart hook that dumped its environment and stdin:

- `$TMUX_PANE` **is present** in the hook subprocess and equals the agent's pane id (e.g. `%176`). tmux injects it into every pane, including user-created splits and hand-launched processes. This is the keystone that makes per-pane, launcher-agnostic capture possible.
- `$CLAUDE_SESSION_ID` is **absent** from the hook environment. The session id is delivered on the hook's **stdin JSON**: `{"session_id": "...", "transcript_path": "...", "cwd": "...", "hook_event_name": "SessionStart", ...}`. The current `for v in CLAUDE_SESSION_ID ...` capture line is therefore dead on Claude 2.x.
- `transcript_path` and `cwd` are provided on stdin for free.

This change builds the recording backbone on those facts. It is the foundation for two later changes (unified `R` resume-all, cold-start recovery) that are explicitly out of scope here.

## Goals / Non-Goals

**Goals:**
- A durable, profile-scoped SQLite store recording, per tmux pane, the agent's native session id (read from hook stdin) keyed by `$TMUX_PANE`.
- Capture works for both AoE-launched and hand-launched agents.
- A reconciler that snapshots volatile per-pane captures into durable per-`(instance, slot)` records while tmux is alive, so the mapping survives reboot.
- Data model for up to 4 tracked agent slots per session, with observe-first adoption and an optional add-agent-pane action.

**Non-Goals:**
- Unified `R`-key restart that resumes every pane (deferred).
- Cold-start manual recovery UI (deferred).
- Migrating `sessions.json` instance config into SQLite (stays JSON).
- Storing conversation content (agents keep their own transcripts; we store only mapping + event stream).
- Changing the existing `status-detection` status-file mechanism or the single primary managed-pane behavior.
- Codex/other-agent capture parity is best-effort only this change (see Open Questions); Claude is the verified, normative target.

## Decisions

### D1: SQLite via `rusqlite` (bundled), not extend `sessions.json`
The data is relational (1 instance × N panes × event stream) and needs upsert-by-key, range queries, and an append-only log. JSON object-per-instance does not fit. Use `rusqlite` with the `bundled` feature so no system libsqlite is required.
- *Alternative considered*: a second JSON file. Rejected — concurrent per-pane upserts and the event stream are awkward and race-prone in JSON.
- *Alternative considered*: `sled`/other embedded KV. Rejected — SQLite is the user's stated preference, is ubiquitous, and gives ad-hoc queryability for debugging.

### D2: Database location and creation via the migration system
`aoe.db` lives in the active profile directory next to `sessions.json`. Schema is created by a new `src/migrations/vNNN_agent_session_store.rs` migration, keeping the main path clean per the project migration convention. The migration is idempotent (`CREATE TABLE IF NOT EXISTS`).

### D3: Capture path — augment the installed status hook, read stdin
The hook command in `src/hooks/mod.rs` gains a branch that, when `$TMUX_PANE` is set, reads stdin JSON, extracts `.session_id`/`.cwd`, and writes a `pane_live` upsert. To avoid embedding a large shell/JSON parser in a one-liner and to keep writes transactional, the hook SHALL shell out to the `aoe` binary itself (a small hidden subcommand, e.g. `aoe __record-pane`) that reads stdin and performs the SQLite upsert. The existing status-file write stays. The legacy `$CLAUDE_SESSION_ID` line is removed.
- *Alternative considered*: keep writing a per-pane file (`/tmp/aoe-hooks/by-pane/<pane>`), have AoE ingest it. Rejected — still on `/tmp` (reboot-lossy for the volatile layer is acceptable, but routing through a subcommand lets the hook write straight to the durable DB and reuse Rust JSON parsing instead of fragile shell `jq`/sed).
- *Alternative considered*: parse JSON in pure shell. Rejected — brittle; `jq` may be absent.

### D4: Reconciler on the existing status-poller tick
`src/tui/status_poller.rs` already ticks per session. The reconciler, per managed session: `tmux list-panes` → for each pane look up `pane_live` by `tmux_pane` → assign a slot deterministically (stable order: the primary `@aoe_agent_pane` is slot 0; remaining panes ordered by tmux pane index) → upsert `agent_slot`. Orphan `pane_live` rows (pane not in any managed session) are deleted. Snapshotting on every tick guarantees the durable record exists before any teardown, so reboot keeps the latest mapping.

### D5: Slot stability
Slot 0 is pinned to the primary managed pane. Additional panes get slots 1..3 by ascending tmux pane index, persisted in `agent_slot`. If a pane disappears and reappears, the reconciler reuses the lowest free slot. This is "good enough" stability for recording; exact identity-preservation across close/reopen is not required for w01/w02.

## Risks / Trade-offs

- **Hook now shells out to `aoe` on every event** → keep `aoe __record-pane` minimal and fast (open DB, single upsert, close); it must never block or error the agent. Wrap in a guard so any failure exits 0.
- **Global hook fires for Claude in non-AoE tmux sessions too** → those writes land in `pane_live` but are GC'd by the reconciler (orphan cleanup) because their pane is not in a managed session. Mild write amplification, self-cleaning.
- **SQLite concurrent writers** (multiple hook subprocesses + AoE reconciler) → use WAL mode and short busy-timeout; all writes are tiny upserts. Acceptable contention.
- **`aoe` binary path in the hook** → resolve the absolute path of the running binary at hook-install time and bake it into the hook command, so the hook works regardless of `$PATH`.
- **Reconcile timing window**: if the machine is killed between a capture and the next tick, the very latest session id may not be in `agent_slot`. → ticks are frequent (seconds); `pane_live` itself can also be persisted (it is a DB table, not `/tmp`), so the durable layer can fall back to `pane_live` if a slot snapshot is missing. Acceptable.
- **New dependency `rusqlite`** increases build time / binary size → bundled SQLite is well-understood; acceptable cost for the capability.

## Migration Plan

1. Add `rusqlite` (bundled) to `Cargo.toml`.
2. Add `vNNN_agent_session_store` migration creating `pane_live`, `agent_slot`, `events` (idempotent) and bump `CURRENT_VERSION`.
3. Ship the new store module + `aoe __record-pane` subcommand.
4. Update hook installation to the new command format; on next agent launch the new hook is written. Old `/tmp/aoe-hooks/<id>/session_id` files are simply abandoned (ephemeral, no migration needed).
- *Rollback*: revert the migration bump and hook format; `aoe.db` can be left in place (unused) or removed.

## Open Questions

- **Codex parity**: Codex uses `CODEX_THREAD_ID` and a different hook/stdin shape; not verified by the spike. This change targets Claude normatively and leaves Codex capture as a best-effort follow-up spike. Confirm whether Codex hooks deliver a thread id on stdin before claiming parity.
- **Should `pane_live` be the durable fallback for resume** (D4 risk) or is `agent_slot` the sole source of truth for the later resume work? Decide when w03 (unified restart) is designed.
- **Add-agent-pane keybinding/CLI surface**: exact trigger (TUI key vs CLI subcommand) to be finalized during apply; must not collide with existing bindings.
