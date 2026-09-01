## 1. RestartPoller worker

- [x] 1.1 Create `src/tui/restart_poller.rs` modeled on `deletion_poller.rs`: `RestartRequest` (session id, cloned `Instance`, profile, `RestartMode`, path kind respawn/recover, pre-read slots + identity origins where applicable), `RestartResult` (session id, updated identity fields, error summary, resulting status), `RestartPoller` with mpsc request/result channels and a single worker thread; register the module in `src/tui/mod.rs`
- [x] 1.2 Move the StayOnHome respawn pipeline body (currently `Action::RespawnAgentPane` in `src/tui/app.rs`) into the worker: slot read fallback handling, single-pane vs fan-out, `resume_all_tracked_panes` / `respawn_single_pane` on the cloned instance, `refresh_session_cache`, building the `RestartResult` from the mutated clone
- [x] 1.3 Move the StayOnHome recovery pipeline body (`recover_instance`) into the worker: store open, slot read, `is_recoverable` re-check (no-op result when stale), `ensure_slot_identity_keys`, `recover_from_slots`, result construction
- [x] 1.4 Wrap the worker's per-request execution in `catch_unwind` so a panic still yields an error `RestartResult`

## 2. Event-loop integration

- [x] 2.1 In `restart_action` (`src/tui/home/input.rs`) / the action handlers, route StayOnHome variants to enqueue: set `restart_in_flight = true` and `Status::Restarting` via `mutate_instance`, snapshot the instance + inputs, send the request; leave Attach variants (`C`/`R`) on the existing synchronous path
- [x] 2.2 Add `restart_poller` field to `HomeView` and an `apply_restart_results()` that merges result fields (`agent_session_id`, `fork_pending`, `resume_token`, `last_error`, status `Starting`/`Error`), clears `restart_in_flight`, and saves; call it from the main loop next to `apply_deletion_results()`
- [x] 2.3 Verify reload-preservation covers the merge window (status/`restart_in_flight`/`last_error` already preserved in `HomeView::reload`); extend the preserved-field list only if the result merge needs more fields

## 3. In-flight gating

- [x] 3.1 Gate restart keys: in `restart_action`, return `None` when the selected instance has `restart_in_flight` set (covers `c`/`r`/`C`/`R`)
- [x] 3.2 Gate attach: Enter/`execute_jump` paths skip instances with `restart_in_flight` (next to the existing `Status::Deleting` checks)
- [x] 3.3 Gate delete: `d` on an in-flight instance is a no-op (no dialog)

## 4. Tests

- [x] 4.1 Unit tests for `RestartPoller` channel plumbing (request in, result out; panic in worker still delivers an error result), following `deletion_poller.rs` test style
- [x] 4.2 Unit tests for gating: `restart_action`/attach/delete are no-ops while `restart_in_flight` is set, re-enabled after `apply_restart_results`
- [x] 4.3 E2E test: `r` on a session returns input control immediately (cursor moves during restart) and the session reaches `Starting`; `d`/Enter during in-flight are rejected. Use `TuiTestHarness` isolation only; never touch the default tmux socket
- [x] 4.4 Compile-check with `cargo check` / `cargo build` during development; do NOT run the full `cargo test` suite on a machine with live AoE sessions (e2e via the isolated harness is the acceptance path)

## 5. Polish

- [x] 5.1 `cargo fmt` and `cargo clippy` clean
- [x] 5.2 Confirm status-bar/help hints still describe `c`/`r` accurately (no copy changes expected; adjust only if a hint claims blocking behavior)
