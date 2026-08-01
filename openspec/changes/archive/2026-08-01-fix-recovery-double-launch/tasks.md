## 1. Single-Launch Rebuild

- [x] 1.1 Give the start path a placeholder mode that creates the session with the default shell instead of the agent command, keeping worktree/sandbox setup, on-launch hooks, and tmux options intact.
- [x] 1.2 Leave the pending fork token and agent auto-confirmation alone in placeholder mode, since no agent was launched.
- [x] 1.3 Use the placeholder mode from cold recovery, and update the ordering comment that assumed the rebuild launches the agent.

## 2. Close the Coverage Gap

- [x] 2.1 Add an opt-in agent stub that exits immediately, leaving the sleep-forever stub as the default.
- [x] 2.2 Cover cold recovery against that stub in resume mode: the session survives and every slot's pane is respawned.
- [x] 2.3 Cover the same in fresh mode, asserting no resume flag is used.

## 3. Verification

- [x] 3.1 Run `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo test --test e2e` with all tmux-touching tests confined to the project harness's private socket.
- [x] 3.2 Run `openspec validate fix-recovery-double-launch --strict`.
- [x] 3.3 Hand the fix back to the acceptance run so cases 2 and 3 can be re-tested against a real agent.
