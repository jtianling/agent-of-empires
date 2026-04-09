## 1. Diagnose and Fix Working Directory

- [x] 1.1 Add debug logging to `split_window_right()` in `src/tmux/session.rs` to trace the exact tmux args (session name, working dir, command) being used
- [x] 1.2 Investigate and fix the root cause: ensure the shell right pane command, when combined with tmux's `-c` flag, correctly sets the working directory. If `-c` alone is insufficient (e.g., due to how tmux dispatches the wrapped `bash -c '...; exec $SHELL'` command), prepend `cd <dir> &&` to the shell command in `build_right_pane_command()` in `src/tui/app.rs` as a defense-in-depth fix
- [x] 1.3 Verify non-shell right pane tools (e.g., "claude") also use the correct working directory (same code path, but confirm)

## 2. Testing

- [x] 2.1 Add an e2e test in `tests/e2e/` that creates a session with a shell right pane pointing at a temp directory, sends `pwd` to the right pane, and asserts the output matches the session's project path
- [x] 2.2 Run `cargo test`, `cargo clippy`, and `cargo fmt` to verify no regressions
