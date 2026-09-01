## Why

Pressing `c` (fresh restart) or `r` (resume restart) on the home view runs the whole restart pipeline synchronously inside the TUI event loop. For Cross Agent Team Claude panes the pipeline includes `auto_confirm_panes`, which polls the pane screen until Claude boots (typically 2-6s, capped at 12s), so every keypress freezes the entire TUI for several seconds. This blocks the natural workflow of restarting one session and immediately moving on to restart or inspect the next one.

## What Changes

- Lowercase `c` / `r` (the `PostRestart::StayOnHome` variants) no longer execute the restart on the event loop. They enqueue a restart request into a new background worker (`RestartPoller`, modeled on the existing `DeletionPoller`) and return immediately.
- Both StayOnHome paths go through the queue: live-session respawn (`Action::RespawnAgentPane`) and cold-start recovery (`Action::RecoverInstance`). The user cannot tell which path a given keypress takes, so both must stop blocking.
- The instance is set to `Status::Restarting` at enqueue time and stays there until the worker's result is applied. The status poller already treats `Restarting` as tier 0 (never polled, never overwritten), and `HomeView::reload()` already preserves in-memory status across the 5s disk reload.
- While a restart is in flight for an instance, the home view rejects: attach (Enter / number jump), delete (`d`), and any further restart keypress (`c`/`r`/`C`/`R`) for that instance. The existing `restart_in_flight` flag becomes the guard.
- The worker performs the same pipeline as today (kill/respawn or recover, auto-confirm, xats reconnect) on a cloned `Instance` + slots, then sends back a result. The event loop merges the result: updated identity fields (`agent_session_id`, cleared `fork_pending` / `resume_token`), per-pane errors into `last_error`, status transition `Restarting` -> `Starting`, then save.
- The queue is a single worker thread: restarts of different instances are serialized, not parallel. Non-blocking input is the goal; total wall time is not.
- Uppercase `C` / `R` (the `PostRestart::Attach` variants) keep today's synchronous behavior unchanged, because auto-confirm must finish before attach.

## Capabilities

### New Capabilities

- `background-restart`: Home-view StayOnHome restarts (`c`/`r`) run on a background serial queue with immediate key return, `Restarting` in-flight gating of attach/delete/restart, and result merge back into the instance on completion.

### Modified Capabilities

<!-- none: the restart pipeline semantics (fan-out, per-pane resume source, fresh identity, per-pane failure isolation, recovery rebuild) are unchanged; only where they execute changes. Existing specs describing C/R attach variants remain accurate. -->

## Impact

- `src/tui/app.rs`: `Action::RespawnAgentPane` / `Action::RecoverInstance` StayOnHome branches move their bodies into the worker; apply-results plumbing added to the event loop.
- `src/tui/restart_poller.rs` (new): request/result channels + worker thread, modeled on `deletion_poller.rs`.
- `src/tui/home/mod.rs`: poller field, `apply_restart_results()`, enqueue helper.
- `src/tui/home/input.rs`: in-flight gating for attach/delete/restart keys.
- `src/session/instance.rs`: no logic changes expected; the moved code calls existing methods on a cloned instance.
- Tests: unit tests for the poller channel plumbing and gating; e2e coverage for non-blocking `c`/`r` and in-flight key rejection.
