## 1. Status Bar Redesign

- [x] 1.1 Rewrite status-left format in `src/tmux/status_bar.rs`: replace `#S` with `#{@aoe_index} #{@aoe_title}`, add conditional `from: #{@aoe_from_title}`, keep single hint `Ctrl+b d detach`
- [x] 1.2 Rewrite status-right format: remove "aoe: Title" prefix, keep conditional branch + conditional sandbox + time
- [x] 1.3 Hide window list: set `window-status-format` and `window-status-current-format` to empty strings in `apply_status_bar()`
- [x] 1.4 Update status bar format test (`test_status_left_format_matches_documented_key_hints`)

## 2. Previous Session Tracking

- [x] 2.1 Add `@aoe_prev_session_` constant and helper functions (get/set) for storing previous session per client in `src/tmux/utils.rs`
- [x] 2.2 Add `@aoe_from_title_` logic: helper to read source session's `@aoe_title` and set it as `@aoe_from_title` on target session
- [x] 2.3 Add `@aoe_index` computation: helper to calculate a session's 1-based index and set it as `@aoe_index` on the target session
- [x] 2.4 Create a unified `track_session_switch()` helper that records prev session, sets from-title, and sets index on the target -- called after every successful switch

## 3. Wire Tracking into All Switch Paths

- [x] 3.1 Wire `track_session_switch()` into `switch_aoe_session()` (n/p and N/P cycle paths)
- [x] 3.2 Wire `track_session_switch()` into `switch_aoe_session_by_index()` (1-9 number jump)
- [x] 3.3 Wire `track_session_switch()` into new `switch_aoe_session_back()` (b toggle)

## 4. Ctrl+b b Back Toggle

- [x] 4.1 Add `switch_aoe_session_back()` function in `src/tmux/utils.rs`: read `@aoe_prev_session_{client}`, validate session exists, switch to it
- [x] 4.2 Add `--back` flag to `SwitchSessionArgs` in `src/cli/tmux.rs` (conflicts with --direction and --index), wire to `switch_aoe_session_back()`
- [x] 4.3 Add back-toggle shell command generator functions (non-nested with hardcoded profile, nested with profile-from-option)
- [x] 4.4 Bind `b` in `setup_session_cycle_bindings()` (non-nested mode)
- [x] 4.5 Bind `b` in `apply_managed_session_bindings()` (nested mode override)
- [x] 4.6 Unbind `b` in `cleanup_session_cycle_bindings()` and `cleanup_nested_detach_binding()`

## 5. Tests

- [x] 5.1 Unit test: `track_session_switch()` sets correct @aoe_prev_session, @aoe_from_title, @aoe_index (mock or in-memory where feasible)
- [x] 5.2 Unit test: `switch_aoe_session_back()` reads prev session and calls switch
- [x] 5.3 Unit test: back toggle with no previous session is a no-op
- [x] 5.4 Verify `cargo fmt`, `cargo clippy`, `cargo test` pass
