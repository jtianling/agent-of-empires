## Why

`Shift+R` is state-aware: it resumes tracked panes when the tmux session is alive and rebuilds a dead but recoverable instance from durable `agent_slot` rows. `Shift+C` is not. It restarts panes clean only while the session is alive, and returns without acting when the selected instance is recoverable.

The result is an asymmetric action matrix with one empty cell. A user who wants to drop a conversation and start the same agents over has no path once the tmux session is gone, which is exactly the state a machine reboot or a lost tmux server leaves behind. The only available action is `R`, which restores the conversation the user wanted to discard.

## What Changes

- Make `Shift+C` state-aware in the same way as `Shift+R`: clean restart for a live session, clean recovery for a recoverable one.
- Carry the restart mode through the cold recovery path so recovered panes launch with no resume flag and no persisted resume token.
- Apply the existing fresh-restart identity transaction to clean recovery: reallocate the pre-allocated session id, drop any pending fork, commit on primary-pane success, roll back otherwise, and clear the stale resume token so a later fork cannot reuse the discarded conversation.
- Restore the saved pane topology during clean recovery exactly as resume recovery does, since layout restoration is independent of whether the conversation is resumed.
- Update the home status hint and help overlay so `C` reads as clean restart or clean recovery according to the selected instance state.
- Add deterministic coverage for `C` routing in both states, and isolated-socket runtime coverage that clean recovery rebuilds the layout while launching every pane without a resume flag.

## Capabilities

### Modified Capabilities

- `tui`: `C` becomes state-aware and routes to clean recovery for a recoverable instance instead of doing nothing.
- `agent-fresh-restart`: fresh restart semantics extend to instances whose tmux session no longer exists.
- `session-layout-recovery`: saved pane topology is restored for recovery in either restart mode.

## Impact

- `src/tui/home/input.rs` for state-aware `C` routing and the existing deleting-state guard.
- `src/tui/app.rs` for the recovery action payload and the recovery handler.
- `src/session/instance.rs` for threading the restart mode through `recover_from_slots` and for reusing the fresh identity transaction on the recovery path.
- `src/tui/components/help.rs` and `src/tui/home/render.rs` for discoverability.
- `src/tui/home/tests.rs` and `tests/e2e/` for routing and runtime coverage.
- No schema change, no external service dependency, and no change to the CLI restart command.
