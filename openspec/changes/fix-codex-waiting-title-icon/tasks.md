## 1. Codex title monitor

- [x] 1.1 Add a Codex-only tmux title monitor that reuses Codex status detection to map waiting to `✋ <session title>` and all other states to the plain session title.
- [x] 1.2 Start or refresh that monitor from the Codex session lifecycle so attached and pre-existing Codex sessions can pick up the waiting-title behavior without changing other session types.

## 2. Title ownership cleanup

- [x] 2.1 Keep the shared TUI status-poller waiting-title behavior aligned with the Codex-only scope so other session types remain untouched.
- [x] 2.2 Update comments and any agent/title metadata that would otherwise describe the old or broader waiting-title behavior.

## 3. Verification

- [x] 3.1 Add focused tests for the Codex title monitor behavior and any attach/start refresh helpers it depends on.
- [x] 3.2 Add or update integration/e2e coverage that verifies a Codex session exposes the raised-hand pane title while waiting and reverts to the plain title afterward.
- [x] 3.3 Run `cargo fmt`, `cargo clippy`, and the relevant test suites.
