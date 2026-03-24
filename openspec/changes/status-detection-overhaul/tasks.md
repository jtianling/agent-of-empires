## 1. Batch Pane Query Infrastructure

- [x] 1.1 Add `PaneInfo` struct (pane_title, current_command, is_dead, pane_pid) and `PaneInfoCache` (HashMap<String, PaneInfo>) in `src/tmux/mod.rs`
- [x] 1.2 Implement `refresh_pane_info_cache()` that runs `tmux list-panes -a -F` with session_name, pane_title, pane_current_command, pane_dead, pane_pid fields, filtered to `aoe_` prefix
- [x] 1.3 Add `get_cached_pane_info(session_name: &str) -> Option<PaneInfo>` accessor
- [x] 1.4 Call `refresh_pane_info_cache()` in the status poller at the start of each poll cycle (alongside existing `refresh_session_cache()`)
- [x] 1.5 Refactor `Session::is_pane_dead()`, `Session::is_pane_running_shell()`, and `Session::get_pane_pid()` to read from `PaneInfoCache` with fallback to direct tmux query
- [x] 1.6 Add unit tests for pane info cache parsing and filtering

## 2. Capture Cache

- [x] 2.1 Add `CaptureCache` struct (HashMap<String, (String, Instant, usize)>) in `src/tmux/session.rs` keyed by session name, storing content + timestamp + line count
- [x] 2.2 Implement `capture_pane_cached(lines: usize)` on `Session` that returns cached content if within 500ms TTL and line count >= requested, otherwise calls `capture_pane()` and updates cache
- [x] 2.3 Update `Session::detect_status()` to use `capture_pane_cached()`
- [x] 2.4 Update resume token extraction in `status_poller.rs` to use `capture_pane_cached()` instead of separate `capture_pane(100)` call
- [x] 2.5 Add unit tests for cache TTL behavior and line count upgrade

## 3. Activity-Gated Polling

- [x] 3.1 Extend session cache to include `window_activity` per session (already partially there -- ensure it's exposed)
- [x] 3.2 Add `last_activity: HashMap<String, i64>` and `last_full_check: HashMap<String, Instant>` to the status poller state
- [x] 3.3 In the poll loop, compare current `window_activity` with `last_activity` for each instance. If unchanged and last full check < 10s ago, skip `update_status()` content detection and reuse previous status
- [x] 3.4 Ensure hook-based agents (Claude, Cursor) bypass the activity gate
- [x] 3.5 Add test for activity gate skip behavior and 10s periodic full check

## 4. Title Fast-Path Detection

- [x] 4.1 Add `detect_status_from_title(title: &str) -> Option<Status>` in `src/tmux/status_detection.rs` that checks for Braille spinner chars (U+2800-U+28FF range) and returns `Some(Status::Running)` if found
- [x] 4.2 Wire title fast-path into `Instance::update_status()`: after hook check, before content detection, read pane title from `PaneInfoCache` and call `detect_status_from_title()`
- [x] 4.3 When title fast-path returns Running, update `last_spinner_seen` timestamp and skip content-based detection
- [x] 4.4 Add unit tests for title fast-path with various spinner chars and non-spinner titles

## 5. Spinner Grace Period

- [x] 5.1 Add `last_spinner_seen: Option<Instant>` to Instance transient state (not serialized)
- [x] 5.2 Update spinner detection points (title fast-path, content-based Running) to set `last_spinner_seen = Some(Instant::now())`
- [x] 5.3 In `update_status()`, when previous status was Running and new detection is non-Running: if `last_spinner_seen` is within 500ms, keep Running status
- [x] 5.4 Add unit/integration test for grace period holding Running during brief spinner gaps

## 6. Spike Detection

- [x] 6.1 Add `spike_start: Option<Instant>` and `pre_spike_status: Option<Status>` to Instance transient state
- [x] 6.2 In `update_status()`, when content-based detection returns Running and previous status was non-Running: set `spike_start = Some(now())`, keep previous status
- [x] 6.3 On subsequent poll, if still Running and spike_start exists: commit to Running, clear spike state
- [x] 6.4 On subsequent poll, if no longer Running: clear spike state, keep non-Running status
- [x] 6.5 Ensure spike detection is bypassed for hook-based and title fast-path Running
- [x] 6.6 Add unit tests for spike confirmation, spike rejection, and bypass scenarios

## 7. Acknowledged Waiting

- [x] 7.1 Add `acknowledged: bool` field to Instance (transient, not serialized, default false)
- [x] 7.2 Set `acknowledged = true` in the TUI attach flow (`src/tui/app.rs`) when user switches to a session
- [x] 7.3 Reset `acknowledged = false` when `window_activity` changes for the instance (in poll loop or `update_status()`)
- [x] 7.4 Apply acknowledged mapping: when detection returns Waiting and `acknowledged == true`, report Idle instead
- [x] 7.5 Apply acknowledged mapping to both content-based and hook-based Waiting
- [x] 7.6 Add unit tests for acknowledge/reset lifecycle and Waiting->Idle mapping

## 8. Integration and Pipeline Wiring

- [x] 8.1 Refactor `Instance::update_status()` to follow the full pipeline order: skip guards -> hooks -> title fast-path -> activity gate -> content detection -> spike -> grace period -> acknowledged -> shell/dead heuristics
- [x] 8.2 Run `cargo fmt` and `cargo clippy` -- fix all warnings
- [x] 8.3 Run `cargo test` -- ensure all existing tests pass
- [x] 8.4 Run existing e2e tests (`cargo test --test e2e`) and verify no regressions
- [x] 8.5 Add golden test fixtures for title fast-path detection scenarios
