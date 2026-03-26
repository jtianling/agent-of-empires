## 1. Monitor State Infrastructure

- [x] 1.1 Add `MonitorSessionState` struct with fields: `last_window_activity`, `last_full_check`, `last_status`, `spike_start`, `pre_spike_status`, `acknowledged`
- [x] 1.2 Refactor `run_notification_monitor()` to maintain a `HashMap<String, MonitorSessionState>` across poll cycles, cleaning up entries for sessions that no longer exist
- [x] 1.3 Refactor `update_notification_options()` to accept and update the state map instead of doing stateless detection each cycle

## 2. Batch Queries and Cache Integration

- [x] 2.1 Call `refresh_session_cache()` and `refresh_pane_info_cache()` at the start of each poll cycle in the monitor
- [x] 2.2 Replace `get_pane_title()` (per-session `list-panes -t`) with `get_cached_pane_info()` lookups from the batch cache
- [x] 2.3 Replace direct `Command::new("tmux").args(["capture-pane", ...])` with `capture_pane_cached()` calls

## 3. Activity Gating in Monitor

- [x] 3.1 Read `window_activity` from session cache for each session at cycle start
- [x] 3.2 Compare with `MonitorSessionState.last_window_activity` to decide whether to skip capture-pane
- [x] 3.3 Implement 10-second periodic full check using `last_full_check` timestamp
- [x] 3.4 Bypass activity gate for hook-based agents (Claude, Cursor) -- always read hook file

## 4. Spike Detection in Monitor

- [x] 4.1 After content-based detection, apply spike detection logic using `MonitorSessionState.spike_start` and `pre_spike_status`
- [x] 4.2 Trust hook-based and title fast-path Running immediately without spike confirmation
- [x] 4.3 Add unit tests for monitor spike detection (transient spike rejected, persistent Running confirmed)

## 5. Acknowledged Waiting in Monitor

- [x] 5.1 Track `acknowledged` flag in `MonitorSessionState`, default `false` for new sessions
- [x] 5.2 Reset `acknowledged` to `false` when `window_activity` changes
- [x] 5.3 Apply acknowledged mapping: if detected Waiting and acknowledged, report as Idle
- [x] 5.4 Update `should_notify_for_instance()` to use the monitor's acknowledged state

## 6. Adaptive Polling Interval

- [x] 6.1 After detection cycle, compute aggregate state: any Running -> 1s, any Waiting -> 2s, all Idle -> 3s
- [x] 6.2 Replace fixed `NOTIFICATION_MONITOR_POLL_INTERVAL` sleep with the computed interval

## 7. Batched Tmux Option Writes

- [x] 7.1 Collect all per-session `@aoe_waiting` values into a Vec
- [x] 7.2 Write all values in a single `tmux` invocation using `\;` command separators
- [x] 7.3 Add fallback to individual writes if batched command fails

## 8. Notification Key Bindings

- [x] 8.1 After computing notification entries, bind keys `1`-`6` in `aoe_notify` key table to switch to corresponding sessions
- [x] 8.2 Key binding run-shell writes session ID to `/tmp/aoe-ack-signal` and switches client
- [x] 8.3 At cycle start, read and delete ack signal file, mark session acknowledged in state map
- [x] 8.4 Clean up all notification key bindings on monitor exit
- [x] 8.5 Add status bar hint showing how to use notification key bindings (e.g., `prefix+1..6`)

## 9. Testing and Verification

- [x] 9.1 Unit tests for `MonitorSessionState` lifecycle (creation, activity reset, spike detection, acknowledged mapping)
- [x] 9.2 Unit tests for adaptive polling interval computation
- [x] 9.3 Unit tests for batched tmux option write construction
- [x] 9.4 Integration test: verify notification monitor detects status changes correctly
- [x] 9.5 Run `cargo fmt`, `cargo clippy`, `cargo test` and fix any issues
