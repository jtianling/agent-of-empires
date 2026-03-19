## 1. Data Model & Dialog Fields

- [x] 1.1 Add `right_pane_tool: Option<String>` to `NewSessionData` struct in `src/tui/dialogs/new_session/mod.rs`
- [x] 1.2 Add `right_pane_tool_index: usize` and right pane tool list (with "none" prepended) to `NewSessionDialog` struct
- [x] 1.3 Add "Right Pane" entry to `FIELD_HELP` array and update field index constants throughout the dialog
- [x] 1.4 Implement Left/Right key handling for the right pane field (cycle through "none" + available tools)
- [x] 1.5 Wire right pane selection into the `DialogResult::Submit(NewSessionData)` path

## 2. Dialog Rendering

- [x] 2.1 Add right pane field rendering in `src/tui/dialogs/new_session/render.rs`, placed directly below the Tool field
- [x] 2.2 Update dialog height calculation to account for the new field
- [x] 2.3 Verify field focus navigation (Up/Down) works correctly with the new field inserted

## 3. Tmux Split-Window Helper

- [x] 3.1 Add a `split_window_right` function in `src/tmux/session.rs` (or `utils.rs`) that executes `tmux split-window -h -t <session> -c <dir> <command>` and sets `remain-on-exit on` on the new pane
- [x] 3.2 For sandboxed sessions, wrap the right pane command with the container's `docker exec` invocation (same pattern as main tool)
- [x] 3.3 Wrap the right pane command with `wrap_command_ignore_suspend` to disable Ctrl-Z

## 4. Session Creation Flow Integration

- [x] 4.1 Pass `right_pane_tool` from `NewSessionData` through `InstanceParams` in `src/session/builder.rs` and `HomeView::create_session()` in `src/tui/home/operations.rs`
- [x] 4.2 After `Instance::start_with_size_opts()` succeeds in `app.rs::attach_session()`, call the split-window helper if `right_pane_tool` is set
- [x] 4.3 Ensure the split happens AFTER `@aoe_agent_pane` is already stored (it is set during `create_with_size`), so it correctly points to the left pane

## 5. Testing & Verification

- [x] 5.1 Add unit tests for the new dialog field (right pane selection cycling, default "none", submission data)
- [x] 5.2 Add unit test verifying `@aoe_agent_pane` is not affected by the split-window operation
- [x] 5.3 Manual test: create session with right pane tool, verify dual-pane layout with correct tools
- [x] 5.4 Manual test: detach from left pane and right pane, verify both return to AoE correctly
- [x] 5.5 Manual test: verify status detection targets left pane after split
- [x] 5.6 Run `cargo fmt`, `cargo clippy`, and `cargo test` to ensure no regressions
