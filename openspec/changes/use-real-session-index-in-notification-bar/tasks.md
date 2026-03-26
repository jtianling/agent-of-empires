## 1. Add real_index to NotificationEntry

- [ ] 1.1 Add `real_index: usize` field to the `NotificationEntry` struct in `src/tmux/notification_monitor.rs`
- [ ] 1.2 Update `ordered_existing_notification_entries()` to compute and assign the 1-based real index from the position in the filtered flatten_tree iteration (after filtering to sessions and existing tmux sessions)

## 2. Use real_index in formatting

- [ ] 2.1 Update `format_notification_text()` to use `entry.real_index` instead of `.enumerate()` sequential numbering
- [ ] 2.2 Verify that `build_notification_entries()` and `visible_notification_entries()` preserve the `real_index` through filtering (no code change expected, just confirm the field flows through)

## 3. Update tests

- [ ] 3.1 Update existing unit tests for `format_notification_text` to construct `NotificationEntry` values with explicit `real_index` fields and assert the correct non-sequential indices in output
- [ ] 3.2 Add a test case verifying index gaps when the current session is excluded (e.g., entries with real_index 1, 2, 3; current session is index 2; output shows [1] and [3])
- [ ] 3.3 Add a test case verifying that `ordered_existing_notification_entries` assigns indices matching the flatten_tree position

## 4. Validation

- [ ] 4.1 Run `cargo fmt` and `cargo clippy` to confirm no warnings
- [ ] 4.2 Run `cargo test` to confirm all tests pass
