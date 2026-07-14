## Context

The home-view `R` key drives `Action::RespawnAgentPane`, which fans out across every tracked pane (`agent_slot` rows) and resumes each from its persisted `native_session_id` via `resume_all_tracked_panes` -> `resume_launch_pane` -> `build_pane_resume_plan`. Inside `build_pane_resume_plan` the decision is:

```rust
let resumed = def.resume.is_some() && is_valid_resume_token(native_session_id);
```

When `resumed` is false the pane still respawns with its full launch context (YOLO flag, hook env, cwd) but without `--resume` -- the `PaneResumeOutcome::DegradedToFresh` path, already covered by tests such as `test_slot_resume_degraded_fresh_keeps_launch_context`. The new "fresh restart" is exactly this path forced on for every pane.

The `r` key currently opens the rename/edit dialog (session rename, or group rename when a group is selected). `e` is unbound in the home view and globally.

## Goals / Non-Goals

**Goals:**
- Add a first-class "restart fresh" action bound to `r` that respawns every tracked pane with no history resume, preserving each pane's agent, cwd, and launch context.
- Keep `R` = resume behavior byte-for-byte identical.
- Move edit/rename (session and group) from `r` to `e`.
- Reuse the existing fan-out; do not duplicate the per-pane respawn/kill machinery.

**Non-Goals:**
- Changing the resume flow, resume-token capture, or cold-start recovery.
- Changing what "edit/rename" does -- only its keybinding moves.
- Adding a CLI subcommand for fresh restart (the existing `aoe session restart` already kills+fresh-starts a single primary pane; out of scope here).

## Decisions

### Decision 1: Parameterize the fan-out with a restart mode enum
Add `enum RestartMode { Resume, Fresh }` (or a bool) threaded through `resume_all_tracked_panes(slots, mode)` -> `resume_launch_pane(..., mode)` -> `build_pane_resume_plan(..., mode)`. In `build_pane_resume_plan`, `Fresh` forces `resumed = false` so no resume flag is appended while the full launch context is still built.

Alternatives considered:
- Duplicate a parallel `fresh_restart_all_panes` function: rejected, it would copy the kill/respawn/failure-isolation logic and drift from the resume path.
- A separate top-level action with no shared core: rejected for the same reason.

### Decision 2: Single-pane fallback forces no stored token
When `slots` is empty, the resume path calls `respawn_agent_pane()` -> `respawn_agent_pane_with_resume(None)`, and `resolved_resume_token(None)` falls back to `self.resume_token`. For the fresh restart this would wrongly reinject history. The fresh single-pane path MUST build the command with `resolved_resume_token` bypassed (pass a sentinel / dedicated fresh respawn that never consults `self.resume_token`).

### Decision 3: One action, a mode; wiring at the input layer
Extend `Action::RespawnAgentPane(String)` to carry the mode, or add `Action::RestartAgentPaneFresh(String)` that shares the app.rs handler body. Input mapping:
- `R` -> resume (existing behavior).
- `r` (no Shift) -> fresh restart. Same `Deleting` no-op guard and `restart_in_flight` guard as `R`.
- `e` -> the block currently under `r`: session RenameDialog, or GroupRenameDialog when a group is selected.

### Decision 4: Reuse `Restarting` status and `restart_in_flight` guard
The fresh restart sets `Status::Restarting` during the fan-out and honors `restart_in_flight` so a second `r` (or an interleaved `R`) is ignored while a restart is in flight, mirroring the resume flow.

### Decision 5: Move BOTH session and group rename to `e`
The `r` key today renames sessions AND groups. Leaving group-rename on `r` while `r` becomes restart-for-sessions would recreate the very inconsistency this change removes. So `e` becomes the universal edit/rename key (session + group); `r` becomes restart (a no-op for a selected group, which has no agent panes).

## Risks / Trade-offs

- [Muscle memory] Users used to `r` = rename will now restart instead. -> Mitigation: help overlay updated; `r` on a group is a harmless no-op, and restart on a session is non-destructive (panes/layout preserved, only agent processes restart fresh).
- [Accidental history loss] Pressing `r` drops conversation history. -> Mitigation: this is the explicit intent of the action; `R` (resume) remains the history-preserving option and keeps the more prominent Shift binding.
- [Single-pane token reinjection bug] The fresh path could accidentally resume via `self.resume_token`. -> Mitigation: Decision 2 bypasses `resolved_resume_token`; add a unit test asserting the fresh single-pane command carries no `--resume`.

## Migration Plan

Pure keybinding + behavior addition; no data/schema migration. Breaking change is limited to the `r`/`e` keybinding swap and is surfaced in the changelog/help overlay.

## Open Questions

None.
