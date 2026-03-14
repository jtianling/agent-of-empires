## 1. Scope-aware cycling

- [x] 1.1 Update tmux session cycling in `src/tmux/utils.rs` to resolve the current managed
  session back to its stored `Instance` and derive the exact in-scope `group_path`.
- [x] 1.2 Filter cycle candidates so grouped sessions only cycle within the same exact group and
  ungrouped sessions only cycle among other ungrouped sessions, while preserving existing visible
  ordering rules.
- [x] 1.3 Keep the hidden `aoe tmux switch-session` path aligned with the new scope rules and
  preserve the existing no-op behavior when no in-scope target exists.

## 2. Nested detach invariants

- [x] 2.1 Verify attach paths continue to seed the immutable AoE return-session target separately
  from the mutable last-detached session tracking used for TUI selection restore.
- [x] 2.2 Update any comments or helper names needed to make the distinction between return target
  and detached-selection target explicit in the tmux binding flow.

## 3. Regression coverage

- [x] 3.1 Add focused unit tests for grouped scope resolution, ungrouped scope resolution, and
  cross-group exclusion in tmux cycling helpers.
- [x] 3.2 Add or update integration/e2e coverage for nested tmux attach where `Ctrl+b j/k` stays
  within group scope and `Ctrl+b d` returns to the AoE TUI after cycling.
- [x] 3.3 Run `cargo fmt`, `cargo clippy`, and the relevant test suites.
