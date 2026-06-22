## Why

After a machine reboot every tmux session is gone, but the durable `agent_slot`
store still holds, per instance, which agents ran in which panes and their
`native_session_id`. Today AoE reloads the instance configs from `sessions.json`
on startup but has no path to rebuild a session's panes and resume each agent from
its persisted id, so the user loses all running agent context across a reboot. This
is the final piece of the w01-w04 agent-session-persistence line (w03 made `R`
resume every tracked pane; w04 makes a cold-started AoE able to do the same for
sessions whose tmux is dead).

## What Changes

- Detect **recoverable** instances on startup / in the home view: an instance that
  has `agent_slot` rows but whose tmux session no longer exists.
- Surface recoverable instances in the home list with a visible marker and a status
  bar hint.
- Add a **manual** per-instance recovery action (one keypress on the focused
  recoverable instance). Cold start is manual, per-session, by decision: AoE does
  not auto-rebuild every session on startup.
- On recovery, rebuild the tmux session (restoring the instance's worktree/sandbox
  context from its config), recreate one pane per persisted slot in slot order, and
  resume each pane from its `agent_slot.native_session_id` via the reusable
  `resume_launch_pane` core introduced by w03. A slot with an empty/invalid id
  degrades to a fresh launch of that pane only.
- After rebuild, write the new tmux pane ids back into `agent_slot.tmux_pane` and
  re-pin `@aoe_agent_pane` to the slot-0 pane so subsequent reconcile/`R` keep working.

## Capabilities

### New Capabilities
- `cold-start-recovery`: detect instances whose durable agent slots survive but whose
  tmux session is dead, mark them recoverable, and provide a manual per-instance
  action that rebuilds the session and resumes each pane from its persisted
  `native_session_id`, writing back the new pane ids.

### Modified Capabilities
<!-- None. w04 is a pure consumer of agent-session-store (reads slots, writes back
     tmux_pane) and reuses w03's resume-launch core without changing their
     spec-level requirements. -->

## Impact

- **Code**: `src/tui/home/` (recoverable detection + marker + recovery action +
  status bar hint), `src/tui/home/input.rs` / `src/tui/app.rs` (recovery action
  wiring), `src/session/instance.rs` (session+pane rebuild that resume-launches each
  slot and writes back pane ids, reusing `resume_launch_pane` / `start` / split
  helpers), `src/db/mod.rs` (`read_slots_for_instance` read, `upsert_agent_slot`
  write-back).
- **Data**: no schema change and no migration; reuses the existing `agent_slot`
  table. Write-back updates `tmux_pane` only.
- **Dependencies**: depends on w03 (`restart-resume-all-panes`) for the per-pane
  `resume_launch_pane` core; already on main.
- **Out of scope**: dead-slot reclaim / `last_seen_at` expiry filtering (recover all
  persisted slots as-is in v1); auto-recovery of all sessions on startup.
