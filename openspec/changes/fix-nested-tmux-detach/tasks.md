## 1. Core Implementation

- [x] 1.1 Add `setup_nested_detach_binding()` helper function in `src/tmux/session.rs` that runs `tmux bind-key d run-shell '...'` with a conditional check on `#{session_name}` prefix
- [x] 1.2 Call `setup_nested_detach_binding()` in `Session::attach()` after a successful `switch-client`
- [x] 1.3 Call `setup_nested_detach_binding()` in `TerminalSession::attach()` after a successful `switch-client`
- [x] 1.4 Call `setup_nested_detach_binding()` in `ContainerTerminalSession::attach()` after a successful `switch-client`

## 2. Verification

- [x] 2.1 Run `cargo clippy` and fix any warnings
- [x] 2.2 Run `cargo test` and verify all tests pass
