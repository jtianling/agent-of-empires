## 1. Add tmux utils helpers

- [x] 1.1 Add private `unset_global_option(option_key: &str)` function in `src/tmux/utils.rs` that runs `tmux set-option -gqu <key>` (mirroring the existing `unset_tmux_session_option` pattern)
- [x] 1.2 Add public `clear_from_title(session_name: &str)` function in `src/tmux/utils.rs` that calls `unset_tmux_session_option(session_name, AOE_FROM_TITLE_OPTION)`
- [x] 1.3 Add public `clear_previous_session_for_client(client_name: &str)` function in `src/tmux/utils.rs` that builds the option key via `client_context_option_key(AOE_PREV_SESSION_OPTION_PREFIX, client_name)` and calls `unset_global_option`

## 2. Call cleanup in TUI attach path

- [x] 2.1 In `src/tui/app.rs`, after `refresh_agent_tmux_options()` and before `update_session_index()` (around line 643-644), call `crate::tmux::utils::clear_from_title(&session_name)` to unset stale from-title on the target session
- [x] 2.2 In the same attach path, inside the `if let Some(client_name) = &attach_client_name` block, call `crate::tmux::utils::clear_previous_session_for_client(client_name)` to unset the stale previous session for the current client

## 3. Testing

- [x] 3.1 Add unit test in `src/tmux/utils.rs` that verifies `clear_from_title` unsets `@aoe_from_title` on a session (set the option, call clear, verify it is gone)
- [x] 3.2 Add unit test in `src/tmux/utils.rs` that verifies `clear_previous_session_for_client` unsets the `@aoe_prev_session_{client}` global option
- [x] 3.3 Run `cargo test` and `cargo clippy` to verify no regressions
