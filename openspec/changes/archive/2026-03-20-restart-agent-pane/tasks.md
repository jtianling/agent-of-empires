## 1. Extract reusable command builder

- [x] 1.1 Extract agent launch command construction from `start_with_size_opts()` in `src/session/instance.rs` into a `build_agent_command()` method that returns the full wrapped command string
- [x] 1.2 Refactor `start_with_size_opts()` to call `build_agent_command()` instead of inline construction
- [x] 1.3 Verify existing tests pass after refactor (`cargo test`)

## 2. Add tmux respawn and pane count support

- [x] 2.1 Add `pane_count()` method to `Session` in `src/tmux/session.rs` that returns the number of panes via `tmux list-panes`
- [x] 2.2 Add `respawn_agent_pane(command)` method to `Session` in `src/tmux/session.rs` that calls `tmux respawn-pane -k -t <@aoe_agent_pane> <command>`
- [x] 2.3 Add scoped process cleanup: a method to kill only the agent pane's process tree (not all panes) using `get_agent_pane_id()` + `get_pane_pid()`

## 3. Add Instance respawn method

- [x] 3.1 Add `respawn_agent_pane()` method to `Instance` in `src/session/instance.rs` that: runs on-launch hooks, calls `build_agent_command()`, calls `Session::respawn_agent_pane()`, calls `apply_tmux_options()`, sets status to `Starting`

## 4. Modify attach-time recovery

- [x] 4.1 In `src/tui/app.rs` attach logic, when pane is dead and session has >1 pane, call `respawn_agent_pane()` instead of `kill-session` + recreate
- [x] 4.2 Keep existing kill+recreate path for single-pane sessions

## 5. Add R keybinding

- [x] 5.1 Add `KeyCode::Char('R')` handler in `src/tui/home/input.rs` that calls `respawn_agent_pane()` on the selected session (or starts it if session doesn't exist)
- [x] 5.2 Add `R` to the help overlay in the TUI

## 6. Testing

- [x] 6.1 Run `cargo fmt`, `cargo clippy`, `cargo test` and fix any issues
- [ ] 6.2 Manual test: create a session, add splits, exit agent, press `R` to verify pane respawn preserves layout
