## 1. Core cycling logic

- [x] 1.1 Add `--global` flag to `aoe tmux switch-session` CLI command
- [x] 1.2 Add `ordered_global_profile_sessions_for_cycle()` that returns all sessions in flatten_tree order, ignoring group collapse state and group scoping
- [x] 1.3 Wire `--global` flag through `switch_aoe_session()` to use the global session list

## 2. Tmux keybindings

- [x] 2.1 Add `session_cycle_global_run_shell_cmds()` to generate shell commands with `--global`
- [x] 2.2 Bind `N` and `P` in `setup_session_cycle_bindings()` (both modes)
- [x] 2.3 Override `N` and `P` in `apply_managed_session_bindings()` (nested mode)
- [x] 2.4 Unbind `N` and `P` in `cleanup_session_cycle_bindings()`
- [x] 2.5 Unbind `N` and `P` in `cleanup_nested_detach_binding()` hook's non-managed branch

## 3. Status bar cleanup

- [x] 3.1 Remove `Ctrl+b n/p switch` from status-left in `configure_status_bar()`
- [x] 3.2 Change `Ctrl+b 1-9 jump` to `Ctrl+b 1-9 space jump` in status-left

## 4. Verification

- [x] 4.1 Run `cargo clippy` and `cargo fmt`
- [x] 4.2 Run `cargo test`
