## 1. Hook reader exposes freshness

- [x] 1.1 Add `HOOK_STATUS_FRESHNESS_WINDOW: Duration = 30s` constant in `src/hooks/mod.rs`
- [x] 1.2 Extend `src/hooks/status_file.rs` with a new reader that returns status plus mtime (e.g. `read_hook_status_with_mtime(instance_id) -> Option<(Status, SystemTime)>`); keep the existing `read_hook_status` as a thin wrapper for callers that don't care about freshness
- [x] 1.3 Add a module helper `is_hook_fresh(mtime: SystemTime) -> bool` that compares to `SystemTime::now()` using the freshness window, with "future mtime" treated as fresh
- [x] 1.4 Unit tests in `status_file.rs`: fresh mtime → `is_hook_fresh == true`; mtime older than window → `false`; future mtime → `true`; reader returns mtime matching the file's actual mtime

## 2. Status poller applies freshness gate

- [x] 2.1 In `src/session/instance.rs::update_status_with_options`, replace the current `read_hook_status` call with the mtime-aware reader; only set `primary_status = Some(hook_status)` when `is_hook_fresh` is true
- [x] 2.2 When hook is stale, add a `tracing::debug!` log with instance id and age in seconds, then fall through to the existing non-hook detection path (title fast-path, content detection, spike, grace)
- [x] 2.3 Preserve the pane-dead override: if pane is dead AND hook would have said Running, still report Error (same as today)
- [x] 2.4 Unit tests in `instance.rs`: (a) fresh hook Running → status Running, content detection skipped; (b) stale hook Running + content says Idle → status Idle; (c) stale hook Waiting + content says Idle + acknowledged=false → status Idle; (d) missing hook file path unchanged

## 3. Notification monitor applies the same gate

- [x] 3.1 Audit `src/tmux/notification_monitor.rs` for every `read_hook_status` / hook-status call site; switch each to use the freshness-aware reader and the same gating logic
- [x] 3.2 Extract the "read hook + check fresh" pattern into a shared helper (in `src/hooks/mod.rs`) so TUI poller and notification monitor call the same function, to prevent future drift
- [x] 3.3 Unit test (or integration test) covering: notification monitor sees stale `running` hook → falls through to content detection → reports Idle

## 4. Documentation and cleanup

- [x] 4.1 Update the tracing comment / module doc in `src/hooks/status_file.rs` to mention the freshness contract
- [x] 4.2 Grep for any other `read_hook_status` callers outside the poller and monitor; ensure they either go through the shared helper or explicitly opt out of freshness gating with a comment explaining why

## 5. Verification

- [x] 5.1 `cargo fmt`
- [x] 5.2 `cargo clippy --all-targets -- -D warnings`
- [x] 5.3 `cargo test` (unit + integration)
- [x] 5.4 Manual sanity check: in a running aoe session, `touch -t <3 hours ago> /tmp/aoe-hooks/<id>/status && echo running > /tmp/aoe-hooks/<id>/status && touch -t <3 hours ago> /tmp/aoe-hooks/<id>/status`, confirm the session flips from Running to Idle within one poll cycle
