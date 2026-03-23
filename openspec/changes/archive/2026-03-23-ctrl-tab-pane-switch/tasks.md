## 1. Register Keybinding

- [x] 1.1 Add `bind-key C-Tab last-pane` in `setup_session_cycle_bindings()` alongside existing `h/j/k/l` pane bindings in `src/tmux/utils.rs`

## 2. Cleanup

- [x] 2.1 Add `unbind-key C-Tab` in `cleanup_session_cycle_bindings()` in `src/tmux/utils.rs`

## 3. Verify

- [x] 3.1 Run `cargo check`, `cargo clippy`, `cargo fmt`, and `cargo test` to confirm no regressions
