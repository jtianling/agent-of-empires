## 1. Extract shared `expanded_groups()` helper

- [x] 1.1 Move `expanded_groups()` from `src/tmux/notification_monitor.rs` to `src/session/groups.rs` as a public free function (or method on a `Vec<Group>` extension)
- [x] 1.2 Update `src/tmux/notification_monitor.rs` to import `expanded_groups` from `src/session/groups.rs` instead of using the local copy
- [x] 1.3 Verify `cargo check` passes with the relocation

## 2. Stabilize index computation in `ordered_profile_session_names()`

- [x] 2.1 Update `ordered_profile_session_names()` in `src/tmux/utils.rs` to call `expanded_groups()` on the input groups before passing to `GroupTree::new_with_groups()` and `flatten_tree()`
- [x] 2.2 Verify that `session_index_in_order()`, `set_target_session_index()`, and `setup_session_cycle_bindings()` all use the stabilized ordering without additional changes

## 3. Add `real_index` to `NotificationEntry`

- [x] 3.1 Add `real_index: usize` field (1-based) to the `NotificationEntry` struct in `src/tmux/notification_monitor.rs`
- [x] 3.2 Compute `real_index` in `ordered_existing_notification_entries()` by tracking position (1-based) as sessions are yielded from the expanded flattened tree, and include it in each `NotificationEntry`
- [x] 3.3 Update `format_notification_text()` to use `entry.real_index` instead of `.enumerate()` sequential numbering

## 4. Update tests

- [x] 4.1 Update unit tests for `format_notification_text` to expect real indices instead of sequential `[1]`, `[2]`, `[3]`
- [x] 4.2 Update unit tests for `session_index_in_order` (if any) to verify indices are stable across group collapse/expand
- [x] 4.3 Update unit tests for `ordered_existing_notification_entries` and `build_notification_entries` to verify `real_index` is correctly populated
- [x] 4.4 Run full test suite (`cargo test`) and fix any breakages from the index change

## 5. Final verification

- [x] 5.1 Run `cargo fmt` and `cargo clippy` to ensure code quality
- [ ] 5.2 Manual sanity check: collapse/expand groups in TUI and verify session indices don't change
- [ ] 5.3 Manual sanity check: verify notification bar `[N]` matches `Ctrl+b N Space` jump target
