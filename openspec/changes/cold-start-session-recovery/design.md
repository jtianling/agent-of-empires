## Context

The agent-session store (w01/w02, on main) persists, per instance, a durable
`agent_slot(instance_id, slot 0..3, agent, native_session_id, cwd, tmux_pane,
last_seen_at)` row per agent pane (`src/db/mod.rs`). These rows survive a machine
reboot. w03 (`restart-resume-all-panes`, on main) added the reusable per-pane
resume-launch core `resume_launch_pane(agent, native_session_id, tmux_pane, cwd)`
(`src/session/instance.rs:1589`) plus `build_pane_resume_command` and the
pane-targeted tmux ops `tmux::respawn_pane_target` / `tmux::kill_pane_process_tree_target`.
w03 also exposes `resume_all_tracked_panes(slots)` which kills+resumes the panes of a
**live** session by their already-recorded `tmux_pane`.

After a reboot the tmux server is empty: `Session::exists()` is false for every
instance even though its `agent_slot` rows are intact. w03's `resume_all_tracked_panes`
cannot help because it targets existing panes; the panes must first be recreated. AoE
already reloads instance configs from `sessions.json` on startup (carrying
worktree/sandbox context) but has no path that rebuilds a dead session's panes and
resumes each agent from its persisted id.

Relevant existing surface this change consumes (no behavioral change to it):
- `Storage::read_slots_for_instance(instance_id)` / `upsert_agent_slot(...)` (`src/db/mod.rs`).
- `Instance::start_with_size_opts` (`src/session/instance.rs:451`) creates the tmux
  session via `session.create_with_size(project_path, cmd, size)` with the slot-0 agent
  command and restores worktree/sandbox context.
- `tmux::split_window_right(session_name, cwd, command)` (used by the `add-agent-pane`
  CLI, `src/cli/session.rs:200`) to create additional panes.
- `resume_launch_pane(...)` for the per-pane resume after panes exist.

## Goals / Non-Goals

**Goals:**
- Classify an instance as recoverable when it has `agent_slot` rows and a dead tmux
  session, surface it in the home view, and offer a manual per-instance recovery action.
- On recovery: rebuild the session (restoring worktree/sandbox), recreate one pane per
  persisted slot in slot order, resume each pane from its `native_session_id`, write the
  new pane ids back into `agent_slot.tmux_pane`, and re-pin `@aoe_agent_pane` to slot 0.
- Reuse w03's `resume_launch_pane` for the per-pane resume; reuse `start` + split helpers
  for session/pane creation.

**Non-Goals:**
- Auto-recovering every session on startup. Cold start is manual and per-session by
  decision.
- Reclaiming dead/stale slots or `last_seen_at` expiry filtering. Recover all persisted
  slots as-is in v1.
- Changing the capture/persist path (hook, `__record-pane`, reconciler) or the
  `agent_slot` schema. No migration.
- Adding a new graceful-exit/scrape path. Recovery is rebuild + direct resume, matching
  w03's Strategy B.

## Decisions

### Decision 1: Recoverable = has slots AND tmux session dead
Detection reads `read_slots_for_instance(&inst.id)` (non-empty) and `tmux_session().exists()`
(false). This is computed where the home view already knows liveness, so the marker and
status-bar hint follow existing focus/render flow. Alternatives considered: persisting an
explicit "needs recovery" flag at shutdown -- rejected, a reboot is not a clean shutdown so
nothing reliably writes such a flag; deriving recoverability live from store + tmux is both
simpler and correct after a crash/reboot.

### Decision 2: Rebuild = recreate session, then resume each pane (not fresh-launch then patch)
Recreate the session with slot 0 and split out slots 1..N, then call `resume_launch_pane`
per slot against the **new** pane ids. Two sub-options for slot 0's first launch:
- (a) Have `start` launch slot 0 fresh, then immediately `resume_launch_pane` it -- one
  redundant fresh process for slot 0.
- (b) Create the session with a placeholder/slot-0-resume command directly.

Choose a single uniform path: create the session shell/primary pane, create the remaining
panes, then run `resume_launch_pane` for **every** slot (including slot 0) against the new
pane ids. Uniform per-slot resume keeps one code path and reuses w03 unchanged; the one
extra respawn of slot 0 is negligible and avoids special-casing the primary pane's command
construction. This mirrors how w03's `resume_all_tracked_panes` already treats every slot
uniformly.

### Decision 3: Capture new pane ids at creation time in slot order, then write back
Each pane id is captured at the moment it is created, in slot order: slot 0 from
`@aoe_agent_pane` (the pane the start path pins), slots 1..N from `split_window_right_capture_pane`
(which returns the new pane's id via `split-window -P -F '#{pane_id}'`). We deliberately do NOT
re-list panes via `reconcile::session_pane_ids`: that orders by `pane_index`, which diverges
from creation/slot order once a session has 3+ panes (every right-split inserts a pane next to
pane 0, so newer panes take lower indexes), so a zip against it would map slots to the wrong
panes. For each (slot, new_pane) run `resume_launch_pane(slot.agent, slot.native_session_id,
new_pane, slot.cwd)`, then `upsert_agent_slot` with the new `tmux_pane`, and set
`@aoe_agent_pane` to the slot-0 pane. Write-back is required so the reconciler and a
subsequent `R` operate on the rebuilt panes rather than stale dead pane ids.

### Decision 4: Reuse `start` for worktree/sandbox restoration
Session recreation goes through the instance's existing start/create path so worktree and
sandbox context restore exactly as a normal start would (no duplicated worktree/sandbox
logic). Per-pane cwd still comes from `agent_slot.cwd` for the resume launch, which can
differ from the instance project path when a pane was opened elsewhere.

### Decision 5: Recovery action is a home-view action, with status-bar hint
The trigger is a home-list keypress on the focused recoverable instance (wired through
`src/tui/home/input.rs` -> `Action` -> `src/tui/app.rs`), not a tmux session keybinding, so
the tmux attach keybinding lifecycle (`setup/cleanup_session_cycle_bindings`) is not
involved. The status-bar hint is gated on the focused instance being recoverable. Pick a key
not already bound in the home view.

### Decision 6: Per-pane degrade and isolation come for free from w03
`resume_launch_pane` already degrades an empty/invalid id (and known-but-no-resume agent) to
a fresh launch, and returns `PaneResumeOutcome::Error` for unsafe/unknown agents or a failed
respawn. Recovery iterates slots, collects outcomes, and never aborts siblings on one
failure -- identical contract to w03's `resume_all_tracked_panes`.

## Risks / Trade-offs

- [Pane index order may not match creation/slot order] -> do not re-list panes by `pane_index`
  (`session_pane_ids` diverges from creation order for 3+ panes); instead capture each new pane
  id at creation time in slot order via `split_window_right_capture_pane`. A slot whose pane
  fails to create degrades to a per-pane error without aborting siblings.
- [`native_session_id` already invalid agent-side (jsonl gc'd)] -> `resume_launch_pane`/the
  agent fails; per-pane isolation degrades that pane to fresh and records the error.
- [Slot 0's extra respawn under the uniform path] -> negligible (one extra process spawn at
  recovery time only); accepted for a single uniform code path.
- [A stale `agent_slot.tmux_pane` from before reboot] -> never targeted: recovery only ever
  resume-launches against freshly created pane ids and overwrites the stale `tmux_pane` on
  write-back.
- [Recovering an instance whose tmux actually came back alive] -> detection re-checks
  `exists()` at action time; a live session is not recoverable, so the action is a no-op.

## Migration Plan

No data migration and no schema change. Behavioral addition only: recoverable detection +
marker + a new home-view recovery action. Rollback = revert the change; the `agent_slot`
store is untouched except for `tmux_pane` write-back, which the reconciler would re-correct
on the next live tick anyway.

## Open Questions

- Exact recovery key and marker glyph/wording -> resolve during implementation against the
  existing home-view keymap and DESIGN.md; must not collide with an existing home binding.
- Whether to later add `last_seen_at`-based pruning of very old slots before listing them as
  recoverable -> deferred (v1 recovers all persisted slots).
- Whether to batch-recover (a "recover all" affordance) later -> out of scope; v1 is manual
  per-session by decision.
