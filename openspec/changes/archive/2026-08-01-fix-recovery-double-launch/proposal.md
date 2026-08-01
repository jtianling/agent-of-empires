## Why

Cold-start recovery does not work with a real agent. It was reported from a manual acceptance run and then reproduced locally: recovering a session leaves the tmux session gone entirely and the TUI reporting `1 pane(s) failed to recover: Failed to respawn pane %N: can't find pane: %N`.

Recovery launches the agent twice. Rebuilding the session starts the primary pane with the agent's command, and the per-slot loop immediately kills that pane and relaunches it with the command the slot actually calls for. The first launch is throwaway by design, but it is not harmless: it runs the agent with the instance's existing `--session-id`, which belongs to the very conversation being recovered. A real agent refuses to start on a conversation id that is already in use and exits at once. The pane dies with it, and since the rebuilt session has only that pane, the session dies too. The per-slot loop then has nothing to respawn.

This affects `R` and `C` equally: both go through the same rebuild. It is not a regression from the recent restart work, which is why the control run on `R` fails identically.

No test caught it because every recovery test in the suite installs an agent stub that sleeps forever. A stub that never exits cannot reach the path where the launched process dies, so the entire cold-recovery test surface has been validating an assumption that no real agent satisfies.

## What Changes

- Rebuild the session with a placeholder shell instead of the agent's command, so recovery launches each agent exactly once, in the per-slot loop that already owns that decision.
- Keep the session-rebuild work that recovery does depend on (worktree and sandbox setup, on-launch hooks, tmux options) unchanged.
- Leave the pending-fork token intact through a placeholder rebuild, since nothing has been launched yet that could consume it.
- Skip agent startup auto-confirmation during a placeholder rebuild; there is no agent to confirm.
- Add an agent stub that exits immediately, and cover cold recovery with it for both restart modes, so the class of failure that hid here is reachable by tests.

## Capabilities

### Modified Capabilities

- `session-layout-recovery`: recovery rebuilds the session with a placeholder and launches each agent once.
- `agent-fresh-restart`: clean recovery inherits the single-launch rebuild.

## Impact

- `src/session/instance.rs` for the placeholder rebuild path and its use from recovery.
- `tests/e2e/harness.rs` for an agent stub that exits immediately.
- `tests/e2e/cold_start_recovery.rs` for coverage of both modes against an agent that refuses to start.
- Fixes cold-start recovery for real agents, which is the state a machine reboot leaves behind.
