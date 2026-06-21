## 1. RED probe test (expose the gating bug first)

- [x] 1.1 Add an attach-scoped e2e in `tests/e2e/` (new file e.g. `attach_reconcile.rs`, registered in `tests/e2e/main.rs`): spawn a managed session, attach to it, while attached inject a `__record-pane` capture (new pane / new native_session_id), then assert `agent_slot` reflects the capture within a bounded time WITHOUT returning to the home view.
- [x] 1.2 Confirm the new test is RED on current `main` (times out / fails), proving the gating bug. Capture the failure output as evidence.

## 2. Implement background reconcile (Plan B)

- [x] 2.1 In `src/tmux/notification_monitor.rs::run_notification_monitor`, add a `last_reconcile: Instant` local and a throttle constant (mirror `status_poller`'s `RECONCILE_INTERVAL` = 750ms).
- [x] 2.2 Inside the loop, when the throttle interval has elapsed, load instances via `Storage::new(profile).load_with_groups()` and call `crate::db::reconcile::reconcile_all(profile, &instances)`; update `last_reconcile`. Keep it best-effort (do not abort the monitor loop on reconcile error; log at debug).
- [x] 2.3 Leave `reconcile_all`, slot assignment, and the home-view driver unchanged (dual idempotent drivers).

## 3. Drive to GREEN + regression

- [x] 3.1 Make the RED probe (1.1) pass with the implementation. (GREEN, ran 3x stable ~1.5-2s, non-flaky)
- [x] 3.2 Run the existing 4 agent-session e2e suites (`agent_session_store`, `pane_session_capture`, `multi_agent_session`, `multi_pane_restart`, 26 cases) and confirm all still green (regression). (27 passed incl. probe)
- [x] 3.3 aoe-tester independent acceptance in isolated HOME (`~/workspace/test`), not touching the real profile; verify no tmux session leak (`tmux ls` diff) and clean up self-opened sessions. (zero leak)

## 4. Wrap-up

- [x] 4.1 `cargo fmt` clean, `cargo clippy --all-targets` clean, lib unit 1163 green, change-relevant e2e (27) green. NOTE: 12 pre-existing e2e reds (default-launch `[all]`-mode drift x11 + codex waiting-icon x1) are unrelated to this change (do not touch `run_notification_monitor`; confirmed by aoe-tester via deterministic isolated repro + git diff/grep); to be tracked separately.
- [x] 4.2 Manual sanity: SKIPPED by decision -- the `attach_reconcile` e2e already spawns a real `monitor-notifications` subprocess and covers the attached-while-poller-idle path; a real-machine recheck would need a binary reinstall + monitor restart (disrupting live sessions), not worth the disruption.
