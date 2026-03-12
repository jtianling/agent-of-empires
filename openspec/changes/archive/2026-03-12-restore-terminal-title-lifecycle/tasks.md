## 1. Restore AoE TUI title lifecycle

- [x] 1.1 Add a small title helper module for setting the stable AoE title and pushing/popping the terminal title stack.
- [x] 1.2 Wire the TUI startup, normal teardown, and panic cleanup paths to save the pre-launch title, set the stable AoE title, and restore the original title on exit.
- [x] 1.3 Reapply the stable AoE title after returning from tmux attach flows in `src/tui/app.rs`.

## 2. Enable tmux title passthrough for managed sessions

- [x] 2.1 Extend the tmux session setup path so AoE-managed agent and paired-terminal sessions enable client title propagation from the active pane title.
- [x] 2.2 Keep pane-title ownership aligned with `sets_own_title`, so agent-emitted title changes and AoE-managed pane title updates both flow through the same tmux title path.
- [x] 2.3 Ensure the tmux title configuration is scoped to AoE-managed target sessions instead of mutating unrelated global tmux defaults.

## 3. Verify runtime behavior

- [x] 3.1 Add focused unit tests for the title helper logic and any tmux option formatting helpers.
- [x] 3.2 Add integration or e2e coverage that verifies AoE-managed sessions expose the expected pane-title and tmux title settings for attach flows.
- [x] 3.3 Run `cargo fmt`, `cargo clippy`, and the relevant test suites covering the terminal title lifecycle change.
