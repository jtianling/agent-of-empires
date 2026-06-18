## Context

The agent session store (w01/w02, already on main) persists, per pane, the mapping
`agent_slot(instance_id, slot 0..3, agent, native_session_id, cwd, tmux_pane, last_seen_at)`
via `src/db/mod.rs`. The reconciler (`src/db/reconcile.rs::reconcile_all`) keeps these
rows up to date on each status-poller tick, with sticky slot assignment (slot 0 pinned
to `@aoe_agent_pane`, max 4).

Current `R` restart (`src/tui/app.rs:494` -> `Action::RespawnAgentPane`) only acts on the
single `@aoe_agent_pane`. The graceful path (`src/session/instance.rs`):
`can_gracefully_restart` -> `initiate_graceful_restart` -> tick-driven `tick_pending_resume`
sends exit keys, waits for the pane to die, then **scrapes** a resume token from pane output
via `extract_resume_token` (regex `claude --resume\s+([0-9a-f-]+)`), then respawns. The
`pending_resume` state machine is a single per-instance `Option<PendingResume>`.

Key observation that drives this change: the token scraped from pane output is the same
uuid that the status hook already captured into `agent_slot.native_session_id`. The hook
reads `{"session_id": ...}` from stdin (`src/cli/record_pane.rs`), and claude prints exactly
`claude --resume <session_id>` on exit. So the entire "exit-keys + scrape" dance exists only
to harvest a token that is now already persisted.

## Goals / Non-Goals

**Goals:**
- One `R` press resumes every tracked agent pane (<=4) of the instance, each from its own
  `native_session_id`.
- Generalize tmux pane operations to an explicit `tmux_pane` target.
- Per-pane failure isolation: a pane that cannot resume degrades to a fresh restart of that
  pane only.
- Provide a reusable "resume-launch one pane by its agent + native_session_id" function that
  w04 (cold-start recovery) can call directly.

**Non-Goals:**
- Cold-start (post-reboot) session recovery. That is w04 (`cold-start-session-recovery`),
  which depends on this change.
- Changing the capture/persist path (hook, `__record-pane`, reconciler). This change is a
  pure consumer of the existing store.
- Reclaiming dead/stale slots or adding `last_seen_at` expiry. Out of scope here; relevant to
  w04's recovery list.
- A separate "restart only the focused pane" keybinding. `R` = all tracked panes by decision.

## Decisions

### Decision 1: Strategy B -- resume from the persisted store id, skip exit-keys + scrape
For each tracked pane, kill the pane process tree and respawn directly with the resume command
built from `agent_slot.native_session_id`. Do **not** send exit keys, do **not** scrape pane
output.

Rationale:
- The token is already persisted; scraping is redundant.
- The existing code already trusts hard-kill + resume: the dead-pane branch
  (`src/session/instance.rs:1006`) respawns with a stored token and no graceful exit, and
  `respawn_agent_pane_with_resume` itself unconditionally calls `kill_agent_pane_process_tree`
  before respawn. The "graceful" C-c only quiesces the agent so it prints the token; the actual
  termination is always a process-tree kill.
- claude/codex journal their session (jsonl / rollout) incrementally and `--resume` reads only
  complete records, so a hard kill loses at most an in-flight partial line, which resume tolerates.

Alternative considered (Strategy A): generalize the existing graceful state machine to N panes
(`Vec<PendingResume>` / `HashMap<slot, PendingResume>`), keeping exit-keys + scrape per pane.
Rejected: it carries the whole multi-phase parallel state machine for no benefit now that the
id is persisted.

### Decision 2: Retire the tick-driven `pending_resume` state machine from the R flow
With Strategy B the restart is a one-step kill+respawn per pane, so there is no exit/scrape phase
to advance across ticks. `R` was the sole trigger of `initiate_graceful_restart` /
`tick_pending_resume` (called only from `src/tui/app.rs:273`), so the state machine becomes dead
code and is removed. `pending_resume` is runtime-only (proven by
`test_pending_resume_is_runtime_only`), so removing it does not touch persistence.

In-flight de-duplication (ignore a second `R` while a restart is running) is preserved with a
lightweight per-instance "restart in flight" flag rather than the old state machine.

### Decision 3: Pane-parameterized tmux operations
`respawn_agent_pane`, `kill_agent_pane_process_tree`, and `send_keys_to_agent_pane`
(`src/tmux/session.rs`) currently resolve `get_agent_pane_id(&self.name)` (the single
`@aoe_agent_pane`). Add `tmux_pane`-target variants (e.g. `respawn_pane_target(pane, cmd, cwd)`)
and have the existing `@aoe_agent_pane` methods delegate to them. The primary pane (slot 0)
keeps working unchanged.

### Decision 4: Reusable per-pane resume-launch function (shared with w04)
Extract a function that, given `(agent, native_session_id, tmux_pane, cwd)`, builds the resume
command (reusing `ResumeConfig.resume_flag` from `src/agents.rs`) and performs kill+respawn for
that one pane, returning a per-pane result. The `R` handler iterates `read_slots_for_instance`
and calls it per slot. w04 calls the same function after recreating panes. This is the shared
core flagged in the roadmap.

### Decision 5: Degrade-to-fresh, not scrape, when no usable id
A pane with no `ResumeConfig`, an empty `native_session_id`, or a failed resume respawn restarts
fresh (no resume flag). No scrape fallback is retained.

## Risks / Trade-offs

- [Hard kill during an in-flight write leaves a partial trailing journal record] -> claude/codex
  resume reads complete records and tolerates a partial tail; this is already the behavior of the
  existing dead-pane respawn path. If a real resume corruption surfaces, a future option is a
  single C-c nudge + short delay before kill, but it is not added pre-emptively (YAGNI).
- [`native_session_id` already invalid on the agent side (jsonl gc'd / cleared)] -> the resume
  respawn or the agent itself fails; per-pane isolation degrades that pane to fresh and records
  the error. (Detailed degrade behavior in specs.)
- [A pane in `agent_slot` whose tmux pane no longer exists] -> respawn against a missing pane
  target errors; treat as a per-pane failure (skip/degrade), do not abort siblings.
- [Removing the graceful state machine could regress a non-R caller] -> verified `R`
  (`RespawnAgentPane`) is the only trigger; `tick_pending_resume` has a single call site.

## Migration Plan

No data migration. Behavioral change only: `R` now restarts every tracked pane instead of one.
`pending_resume` and the graceful exit-key/scrape code paths used solely by `R` are removed; the
status-poller token-capture path and `Instance.resume_token` persistence are left untouched in
this change (they are not on the `R` critical path under Strategy B). Rollback = revert the change;
no stored state is altered.

## Open Questions

- Whether to later remove the now-unused `Instance.resume_token` capture/persist infrastructure
  entirely. Deferred: it is harmless and out of scope here; revisit after w04.
- Exact wording/UX for surfacing a per-pane resume failure (which degraded to fresh). Resolve
  during implementation; minimum is an instance-level error note.
