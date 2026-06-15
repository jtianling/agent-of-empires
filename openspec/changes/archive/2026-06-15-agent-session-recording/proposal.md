## Why

Today AoE only "manages" a single agent pane per session, and its only durable record of an agent's identity is a per-instance `session_id` file under `/tmp` written by a hook that depends on a `$CLAUDE_SESSION_ID` environment variable. A spike against Claude 2.1.177 proved that env var no longer exists (the session id is delivered on the hook's stdin JSON instead), and `/tmp` is wiped on reboot. As a result: agents the user starts by hand inside a shell pane are invisible to AoE, multiple agents in one session cannot be told apart, and nothing survives a machine restart. This change builds the durable recording backbone (the foundation that later unified-restart and cold-start-recovery work depends on).

## What Changes

- Add a local SQLite store (new `rusqlite` dependency, bundled) under the app data dir, created/migrated through the existing `src/migrations/` system. It records per-pane agent session identity and a status/event stream. It does NOT store conversation content (agents keep their own transcripts).
- Add a `$TMUX_PANE`-keyed capture path to the installed agent status hook (tmux injects `$TMUX_PANE` into every pane, including user-created splits and hand-launched agents). The hook reads the native session id from its **stdin JSON** (`.session_id`) rather than the `$CLAUDE_SESSION_ID` env var, and writes a per-pane record to the SQLite store. The existing per-instance status file (used by `status-detection`) is left untouched. The legacy `$CLAUDE_SESSION_ID`/`$CODEX_SESSION_ID` session-id-from-env capture line (already non-functional on Claude 2.x) is removed as superseded.
- Add a reconciler (driven by the existing status-poller tick) that enumerates each managed session's tmux panes, resolves each pane's captured native session id by `$TMUX_PANE`, and snapshots `(instance, slot, agent, native_session_id, cwd)` into the durable table; it also garbage-collects volatile capture rows that do not belong to any known session.
- Extend the session data model so a session may track up to **4** agent panes ("slots") in addition to its existing single primary managed pane. AoE adopts agents that appear in any pane of a managed session (observe-first), and additionally exposes an optional "add agent pane" action. The primary managed-pane model and its APIs are preserved; slots are additive tracking metadata.

Explicitly OUT OF SCOPE (deferred to follow-up changes): the unified `R`-key restart that resumes every pane, and the cold-start manual recovery UI. This change only records and models; it does not change restart/resume behavior or add recovery UI. `sessions.json` remains the store for instance configuration (not migrated to SQLite).

## Capabilities

### New Capabilities
- `agent-session-store`: SQLite-backed durable store holding per-pane agent session records (instance, slot, agent, native session id, cwd, last-seen) and an append-only status/event stream; created and migrated via the existing migration system.
- `pane-session-capture`: the hook + reconciler pipeline that captures each pane's native session id keyed by `$TMUX_PANE` (read from stdin JSON), works for both AoE-launched and hand-launched agents, and snapshots volatile per-pane captures into durable per-slot records.
- `multi-agent-session`: a managed session may track up to 4 agent panes; AoE adopts agents appearing in any pane and offers an optional add-agent-pane action.

### Modified Capabilities

None. This change is purely additive recording infrastructure. The agent status-detection hook (in `src/hooks/mod.rs`, distinct from the `hooks` lifecycle capability) gains an additional `$TMUX_PANE`-keyed capture path; the existing per-instance status-file detection (`status-detection` capability) and the single managed-pane model (`session-management` capability) are left intact. The new behavior is captured entirely under the three new capabilities above. The unified-restart and cold-start-recovery work that will consume these records (and may then modify `session-management` / restart capabilities) is deferred to follow-up changes.

## Impact

- Dependencies: add `rusqlite` (bundled SQLite) to `Cargo.toml`.
- Code:
  - `src/hooks/mod.rs` (hook command construction), `src/hooks/status_file.rs` (capture path / read helpers).
  - New module for the SQLite store (e.g. `src/db/` or `src/session/store.rs`).
  - `src/migrations/` (new migration creating the schema).
  - `src/session/instance.rs` and `src/session/mod.rs` (multi-slot model, adoption helpers).
  - `src/tui/status_poller.rs` (reconciler tick hook-in) and status-detection plumbing in `src/tmux/`.
  - `src/agents.rs` (align Codex capture with the new stdin-based approach if applicable).
- Data: new `aoe.db` SQLite file in the app data dir; `sessions.json` schema unchanged.
- Backward compatibility: the changed hook format and capture path are a deliberate break; old `/tmp/aoe-hooks/<id>/session_id` files are abandoned (no migration needed since they are ephemeral).
