## 1. Profile Resolution - Directory Only

- [x] 1.1 Remove tmux session name branch from `resolve_profile()` in `src/session/mod.rs` - delete the `TMUX` env var check and `get_tmux_session_name()` call, always use directory-based resolution
- [x] 1.2 Remove `get_tmux_session_name()` function (now unused)
- [x] 1.3 Update unit tests in `src/session/mod.rs` for the new resolution logic

## 2. Multi-Instance Tracking

- [x] 2.1 Add `register_instance(profile)` function in `src/session/mod.rs` that writes a PID file to `<profile_dir>/.instances/<pid>`
- [x] 2.2 Add `unregister_instance(profile)` function that removes the PID file on exit
- [x] 2.3 Add `cleanup_stale_instances(profile)` function that scans `.instances/` and removes PIDs for dead processes (using `kill(pid, 0)`)
- [x] 2.4 Add `has_other_instances(profile)` function that checks if any other live aoe instances are using the profile
- [x] 2.5 Update `cleanup_empty_profile()` to check `has_other_instances()` before deleting

## 3. Startup and Exit Integration

- [x] 3.1 Call `register_instance()` and `cleanup_stale_instances()` in `main.rs` after profile resolution
- [x] 3.2 Call `unregister_instance()` in `main.rs` before `cleanup_empty_profile()` on TUI exit
- [x] 3.3 Ensure unregister runs even if TUI returns an error (use a guard or explicit cleanup)

## 4. New Session Dialog - Remove Profile Field

- [x] 4.1 Remove `profile`, `available_profiles`, `profile_index` fields from `NewSessionDialog` struct
- [x] 4.2 Remove profile cycling input handling (Left/Right on profile field)
- [x] 4.3 Remove profile rendering from the dialog
- [x] 4.4 Update `NewSessionDialog::new()` constructor to no longer accept profile list
- [x] 4.5 Set `NewSessionData.profile` from the parent HomeView's current profile unconditionally
- [x] 4.6 Update callers of `NewSessionDialog::new()` in `src/tui/home/input.rs`

## 5. Profile Deletion Error Surfacing

- [x] 5.1 In `ProfilePickerDialog`, propagate `delete_profile()` errors to the dialog's `error` field instead of ignoring them
- [x] 5.2 Verify error display in the profile picker UI

## 6. Testing

- [x] 6.1 Add unit tests for `register_instance`, `unregister_instance`, `has_other_instances`, `cleanup_stale_instances`
- [x] 6.2 Update existing profile management integration tests in `tests/profile_management.rs`
- [x] 6.3 Run `cargo fmt`, `cargo clippy`, and `cargo test` to verify everything passes
