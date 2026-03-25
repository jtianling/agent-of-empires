## 1. Add public ack signal writer

- [x] 1.1 Add `pub fn write_ack_signal(instance_id: &str) -> Result<()>` to `src/tmux/notification_monitor.rs` that writes the instance_id to the ack signal file path (reusing `ack_signal_path()`)
- [x] 1.2 Export `write_ack_signal` from the `tmux` module so `utils.rs` can call it

## 2. Add instance_id resolution helper

- [x] 2.1 Add a helper function in `src/tmux/utils.rs` that takes a target session name and a slice of `Instance` and returns `Option<String>` (the matching instance_id) by comparing against `Session::generate_name(&instance.id, &instance.title)`

## 3. Integrate ack into quick-switch functions

- [x] 3.1 In `switch_aoe_session` (direction-based cycle), after `switch_client_to_session` succeeds, resolve the target session's instance_id and call `write_ack_signal`
- [x] 3.2 In `switch_aoe_session_by_index` (number jump), after `switch_client_to_session` succeeds, resolve the target session's instance_id and call `write_ack_signal`
- [x] 3.3 In `switch_aoe_session_back` (back toggle), after the switch succeeds, resolve the target session's instance_id and call `write_ack_signal`

## 4. Remove notification keybinding infrastructure

- [x] 4.1 Remove constants `NOTIFICATION_TRIGGER_KEY`, `NOTIFICATION_KEY_TABLE`, and `NOTIFICATION_HINT_OPTION` from `notification_monitor.rs`
- [x] 4.2 Remove `setup_notification_key_bindings()` function and its call site in the monitor loop
- [x] 4.3 Remove `cleanup_notification_key_bindings()` function and its call sites in the monitor shutdown/cleanup paths
- [x] 4.4 Remove `notification_binding_hint()` function
- [x] 4.5 Remove the `NOTIFICATION_HINT_OPTION` session option write from `build_notification_session_updates()` (the `@aoe_notification_hint` push)
- [x] 4.6 Remove the `@aoe_notify_target_*` and `@aoe_notify_instance_*` session option writes from `build_notification_session_updates()` (the per-index loop that pushes target/instance options)
- [x] 4.7 Remove `NOTIFICATION_HINT_OPTION` unset calls from the monitor cleanup/shutdown paths (where sessions are cleaned up on exit)
- [x] 4.8 Remove the `MAX_NOTIFICATION_BINDINGS` constant if it is no longer referenced after the above removals

## 5. Verify and clean up

- [x] 5.1 Run `cargo clippy` and fix any warnings from dead code or unused imports
- [x] 5.2 Run `cargo fmt`
- [x] 5.3 Run `cargo test` and verify all tests pass
- [x] 5.4 Verify status bar still displays notification entries (session icons and titles) without the hint text
