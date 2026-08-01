## Why

Today the `R` restart only respawns the single AoE-managed agent pane (`@aoe_agent_pane`). In a multi-agent session where several panes each run their own agent, pressing `R` leaves the other agents un-resumed. The `agent-session-store` work (w01/w02) now persists each pane's `native_session_id` in `agent_slot`, so a single `R` can resume every tracked agent pane using its own session id.

## What Changes

- `R` restart fans out from the single `@aoe_agent_pane` to **all tracked agent panes** of the instance (`agent_slot` rows, up to 4).
- Each pane is resumed using its persisted `agent_slot.native_session_id` to build the agent's resume command directly (`claude --resume <id>`, `codex resume <id>`), in the pane's own `agent_slot.cwd`.
- The store-id path **replaces the graceful "exit-keys + scrape token from pane output" dance for the `R` flow**: with the session id already persisted, the system kills the pane process tree and respawns with the resume command in one step. No tick-driven exit/scrape state machine is needed for tracked panes.
- tmux pane operations (`respawn_agent_pane`, `kill_agent_pane_process_tree`, `send_keys_to_agent_pane`) are generalized from the hardcoded `@aoe_agent_pane` target to an arbitrary `tmux_pane`.
- Per-pane failure isolation: a pane whose resume fails (or that has no usable `native_session_id`, or whose agent has no `ResumeConfig`) degrades to a fresh restart of that pane only, without affecting the other panes.
- Status reflects an aggregated `Restarting` state while the multi-pane restart is in flight.
- Behavior change (not a data/format break): `R` now restarts every tracked agent pane instead of just one. The single-pane graceful-scrape path is superseded by the store-based resume for the `R` keybinding.

## Capabilities

### New Capabilities
- `multi-pane-resume-restart`: pressing `R` resumes every tracked agent pane of the instance, each from its persisted `native_session_id`, with per-pane failure isolation and pane-parameterized tmux operations.

### Modified Capabilities
- `agent-resume-restart`: the `R` restart entry point fans out to all tracked panes; the resume token is sourced from the persisted store (`agent_slot.native_session_id`) rather than scraped from pane output; the single-pane tick-driven exit-key/scrape state machine is no longer used by the `R` flow.

## Impact

- `src/tui/app.rs`: `Action::RespawnAgentPane` handler reads `read_slots_for_instance` and drives the per-pane restart.
- `src/session/instance.rs`: resume-launch logic parameterized per pane/agent; the single `pending_resume` scrape state machine retires from the `R` path.
- `src/tmux/session.rs`: `respawn_agent_pane` / `kill_agent_pane_process_tree` / `send_keys_to_agent_pane` accept an explicit `tmux_pane` target instead of resolving `@aoe_agent_pane`.
- `src/db/mod.rs`: consumes existing `read_slots_for_instance` + `AgentSlot` (no schema change).
- `src/agents.rs`: existing `ResumeConfig.resume_flag` / `session_id_flag` reused to build per-agent resume commands.
- Acceptance: new e2e coverage in `tests/e2e/` (multi-pane restart asserting each pane respawns with its `--resume <native_session_id>`).
