## Context

The home screen currently exposes two restart-family actions:

- `Shift+R` routes on recoverability. A live session resumes through `RespawnAgentPane(id, RestartMode::Resume)`; a recoverable instance goes through `RecoverInstance(id)` into `Instance::recover_from_slots`.
- `Shift+C` restarts clean through `RespawnAgentPane(id, RestartMode::Fresh)`, but returns early when `is_recoverable(id)` holds (`src/tui/home/input.rs`).

`recover_from_slots` already reconstructs everything a clean restart needs: it recreates the tmux session and one pane per durable slot, restores the saved window layout from the layout snapshot, and relaunches each slot's own agent in its own cwd. The only reason it cannot serve a clean restart is that its per-pane launch is hardcoded to `RestartMode::Resume`.

This means the missing behavior is not a new execution core. It is a parameter that never got threaded through, plus the identity bookkeeping that the live fresh path already performs and the recovery path never needed.

That bookkeeping matters. `begin_fresh_identity` exists because a fresh restart must not leave the instance pointing at the conversation it just discarded: it reallocates the pre-allocated `--session-id`, drops a pending fork, and clears the stale `resume_token` once the primary pane respawns, rolling back when the respawn never starts. Recovery has never had a fresh mode, so it has no equivalent, and adding one without this transaction would let a later fork resurrect the discarded conversation.

## Goals / Non-Goals

**Goals:**

- Give `Shift+C` one state-aware "start these agents over" meaning that works whether or not the tmux session survived.
- Reuse the existing recovery core (session rebuild, slot-to-pane mapping, layout restoration, per-pane failure isolation) unchanged.
- Apply the same identity transaction to clean recovery that clean restart already applies.
- Keep `Shift+R` behavior byte-for-byte unchanged.

**Non-Goals:**

- Recover instances that have no durable `agent_slot` records.
- Automatically recover or restart anything at AoE startup.
- Change the CLI `session restart` command.
- Add a confirmation dialog for either clean action.
- Preserve or restore any xats registration identity. That is a separate change.

## Decisions

### Decision 1: Route `Shift+C` at the home input boundary, mirroring `Shift+R`

When the selected instance is recoverable, `C` emits the recovery action in fresh mode. Otherwise it emits the existing `RespawnAgentPane(id, RestartMode::Fresh)`, subject to the existing `Deleting` no-op guard. The two execution cores stay separate while presenting one user intent, which is the structure `R` already established.

Alternative considered: detect the missing session inside the app-level fresh restart handler. Rejected for the same reason it was rejected for `R`: the home model already owns recoverability and contextual hints, and routing there keeps both branches explicitly testable.

### Decision 2: Carry the mode on the recovery action rather than adding an action variant

`Action::RecoverInstance` gains a `RestartMode` payload and `recover_from_slots` gains a corresponding parameter, replacing the hardcoded `RestartMode::Resume` at its per-pane launch site. One recovery path serves both keys.

Alternative considered: a separate `RecoverInstanceFresh` action and a parallel `recover_from_slots_fresh`. Rejected because the two would differ by one argument while duplicating session rebuild, layout remapping, slot write-back, and error aggregation.

### Decision 3: Clean recovery reuses the fresh identity transaction

Clean recovery performs the same identity handling as clean restart: reallocate the pre-allocated session id, drop any pending fork, commit only when the primary slot relaunches, roll back otherwise, and clear the stale `resume_token` on success.

This is the decision most likely to be lost during implementation, because recovery visibly "works" without it. The failure it prevents is delayed and confusing: the user clears a conversation, forks the session later, and the fork resumes the conversation that was supposed to be gone.

The durable `agent_slot.native_session_id` values are not consulted for launching in fresh mode. They are left in place rather than blanked, because the pane capture and reconcile chain rewrites them once the relaunched agents register their new native sessions, and an empty slot row would make the instance briefly look unrecoverable.

### Decision 4: Layout restoration is independent of restart mode

The saved topology is remapped and applied to the newly created panes before attach, exactly as in resume recovery, including the existing safe degradation when the snapshot is absent, stale, or rejected by tmux. Pane geometry has nothing to do with whether a conversation is resumed, so it must not become a second behavior to maintain.

### Decision 5: No confirmation dialog

`C` on a live session already discards conversations without confirmation. Adding a prompt only on the recovery branch would make the more destructive-feeling case behave differently from the established one. The contextual status hint carries the distinction instead, and the existing in-flight guard still prevents a duplicate press from starting a second fan-out.

The primary-pane relaunch inside the recovery path stays a fresh launch in fresh mode, so the session rebuild and the uniform per-pane relaunch agree rather than starting the primary agent from a resumed command and immediately replacing it.

## Risks / Trade-offs

- [A user presses `C` intending `R` and loses recoverable conversations] -> The contextual status hint names the branch for the selected instance, and help lists `C` as the clean action. No new prompt is introduced, consistent with the live-session behavior that already ships.
- [Fresh identity transaction omitted on the recovery branch] -> Covered by a dedicated task and by unit coverage asserting the stored resume token is cleared and a pending fork is dropped after a clean recovery.
- [Layout behavior silently diverges between the two recovery modes] -> Runtime coverage asserts nested geometry survives clean recovery, not only resume recovery.
- [Per-pane failure isolation regresses while threading the new parameter] -> Existing per-pane error aggregation is asserted in both modes.
- [Slot rows briefly reference stale native session ids after a clean recovery] -> Accepted. The reconcile chain refreshes them, and recoverability is intentionally preserved during the window.

## Migration Plan

No data migration. The change is behavioral and ships with the binary. `Shift+C` on a recoverable instance changes from doing nothing to performing clean recovery, which is additive from the user's perspective; no previously working action changes meaning. Rollback is the previous binary.

## Open Questions

- None blocking.
