## 1. Add `refresh-bindings` CLI subcommand

- [x] 1.1 Add `RefreshBindings` variant to the tmux CLI subcommand enum in `src/cli/tmux.rs` with `--client-name` option
- [x] 1.2 Implement `refresh_bindings()` in `src/tmux/utils.rs`: check current session for given client, if managed call bind-key d/j/k via `Command::new("tmux")`, if not managed restore d to detach-client and unbind j/k
- [x] 1.3 Wire the new subcommand to call `refresh_bindings()` in the CLI handler

## 2. Fix hook generation in `setup_nested_detach_binding()`

- [x] 2.1 Rewrite the hook command in `setup_nested_detach_binding()` to use `run-shell "<aoe_bin> tmux refresh-bindings --client-name #{client_name}"` as the true-branch instead of embedding shell commands
- [x] 2.2 Simplify the `if-shell` test to use `#{m:aoe_*,#{session_name}}` format conditional instead of `display-message | grep` pipeline
- [x] 2.3 Remove `shell_escape()` usage from hook generation (no longer needed for the hook command)
- [x] 2.4 Remove `nested_detach_run_shell_cmd()` and `nested_cycle_run_shell_cmd()` functions (replaced by `refresh_bindings()`)

## 3. Fix `switch-client` targeting in attach paths

- [x] 3.1 Pass `client_name` with `-c` flag to `switch-client` in `Session::attach()` (`src/tmux/session.rs`)
- [x] 3.2 Pass `client_name` with `-c` flag to `switch-client` in `TerminalSession::attach()` (`src/tmux/terminal_session.rs`)
- [x] 3.3 Pass `client_name` with `-c` flag to `switch-client` in `ContainerTerminalSession::attach()` (`src/tmux/terminal_session.rs`)

## 4. Verify and test

- [x] 4.1 Run `cargo clippy` and `cargo fmt` to ensure code quality
- [x] 4.2 Run `cargo test` to verify existing tests pass
- [x] 4.3 Verify the hook installs successfully (no syntax error from `set-hook`)
