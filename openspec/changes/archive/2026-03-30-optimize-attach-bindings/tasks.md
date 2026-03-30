## 1. Batch tmux commands via source-file

- [ ] 1.1 Refactor `setup_session_cycle_bindings()` in `src/tmux/utils.rs` to collect all bind-key and set-option commands into a `Vec<String>`, write them to a `NamedTempFile`, and invoke `tmux source-file <path>` once instead of individual `Command::new("tmux")` calls
- [ ] 1.2 Refactor `tag_sessions_with_profile()` to return command strings instead of executing them, so they can be included in the same batch
- [ ] 1.3 Refactor `setup_number_jump_bindings()` to return command strings instead of executing them, so they can be included in the same batch
- [ ] 1.4 Refactor `cleanup_session_cycle_bindings()` and `cleanup_number_jump_bindings()` to batch all unbind-key commands into a single `tmux source-file` invocation

## 2. Move binding setup before raw-mode-disabled window

- [ ] 2.1 Remove the `setup_session_cycle_bindings(profile)` call from `Session::attach()` in `src/tmux/session.rs`, making `attach()` only check existence + run `tmux attach-session`
- [ ] 2.2 Add `setup_session_cycle_bindings(profile)` call in `App::attach_session()` in `src/tui/app.rs` before the `with_raw_mode_disabled` block (after the existing `update_session_index` call)
- [ ] 2.3 Ensure the CLI attach path (`src/cli/session.rs`) still calls `setup_session_cycle_bindings(profile)` before `tmux attach-session`

## 3. Verification

- [ ] 3.1 Run `cargo fmt` and `cargo clippy` with no warnings
- [ ] 3.2 Run `cargo test` and verify all tests pass
- [ ] 3.3 Manual test: launch AoE, enter a session, verify no command-line flash and all keybindings work (Ctrl+b b, Ctrl+., Ctrl+,, Ctrl+b 1, h/j/k/l)
