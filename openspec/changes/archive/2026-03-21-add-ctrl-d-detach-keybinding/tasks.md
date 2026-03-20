## 1. Core Binding Implementation

- [x] 1.1 In `apply_managed_session_bindings()`, add a root-table `C-d` binding using `if-shell` to check session name: if `aoe_*`, run `detach_run_shell_cmd()`; otherwise `send-keys C-d`
- [x] 1.2 In `cleanup_nested_detach_binding()`, add `unbind-key -T root C-d` alongside the existing prefix cleanup
- [x] 1.3 In `cleanup_session_cycle_bindings()`, add `unbind-key -T root C-d`
- [x] 1.4 In `refresh_bindings()` non-managed branch, add `unbind-key -T root C-d`

## 2. Status Bar Update

- [x] 2.1 Update the status bar hint in `src/tmux/status_bar.rs` to show `Ctrl+d` instead of `Ctrl+b d` for detach

## 3. Testing

- [x] 3.1 Add unit test verifying the root-table `C-d` binding command is generated with `if-shell` guard
- [ ] 3.2 Manually verify `Ctrl+d` returns to AoE from a managed session (nested mode)
- [ ] 3.3 Manually verify `Ctrl+d` sends EOF in a non-managed session
- [x] 3.4 Run `cargo fmt`, `cargo clippy`, `cargo test`
