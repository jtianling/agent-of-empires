## Context

The home-view restart keys map to four variants: `r`/`R` resume, `c`/`C` fresh; lowercase stays on the home list (`PostRestart::StayOnHome`), uppercase attaches (`PostRestart::Attach`). All four run synchronously inside `App::handle_action` on the TUI event loop (`src/tui/app.rs:434` and `recover_instance`). The dominant cost is `Instance::auto_confirm_panes` (`src/session/instance.rs`): for non-sandboxed Claude panes in Cross Agent Team mode it polls `tmux capture-pane` until Claude's startup screens are answered and the input prompt appears (2-6s typical, 12s cap), then submits the xats `reconnect` turn. While this runs, the event loop neither reads input nor redraws.

The codebase already has the exact architecture needed: `DeletionPoller` and `CreationPoller` (`src/tui/deletion_poller.rs`, `creation_poller.rs`) are request-channel/worker-thread/result-channel pollers whose results are applied once per event-loop tick. Two facts make restart safe to move onto that pattern:

- `Status::Restarting` is polling tier 0 in `status_poller.rs`: the status poller never polls or overwrites an instance in that state.
- `HomeView::reload()` (`src/tui/home/mod.rs:321`) explicitly preserves in-memory `status`, `last_error`, and `restart_in_flight` across the periodic disk reload, so a parked `Restarting` row survives.

`Instance::restart_in_flight` exists today but is set and cleared within one synchronous call, so it currently guards nothing; it becomes meaningful once restarts overlap with input handling.

## Goals / Non-Goals

**Goals:**

- `c`/`r` return to the event loop in well under a keypress-to-keypress interval (no multi-second freeze), for both the live-respawn and cold-start-recovery paths.
- The user can queue restarts on several sessions back to back and keep navigating.
- In-flight instances are protected from conflicting operations (attach, delete, another restart).
- Restart outcomes (identity updates, per-pane errors) land in the instance exactly as they do today.

**Non-Goals:**

- Parallel restarts. A single serial worker is deliberate: wall-clock time is not the complaint, blocked input is.
- Changing `C`/`R`. Auto-confirm must complete before attach (keystroke injection would race the user's typing), so the attach variants stay synchronous.
- Changing any restart semantics inside `src/session/instance.rs` (fan-out, per-pane resume source, fresh identity transaction, recovery rebuild). The pipeline moves, it does not change.
- Cross-process visibility of in-flight state. `restart_in_flight` stays in-memory per TUI process, same as today.

## Decisions

### D1: Clone-and-merge worker, following `CreationPoller`

The worker receives a cloned `Instance` plus its pre-read `agent_slot` rows and profile, runs the existing pipeline (`resume_all_tracked_panes` / `respawn_single_pane` / `recover_from_slots` + `auto_confirm`), and sends back a `RestartResult` carrying the fields the pipeline mutates: `agent_session_id`, `fork_pending`, `resume_token`, per-pane error summary, and (for recovery) the rewritten slot pane ids already persisted by the pipeline itself. The event loop merges those fields via `mutate_instance`, sets `Starting`, and saves.

Why not share the instance under a lock: every other background path in this TUI (status, deletion, creation) uses message passing and single-point merge; a lock would introduce a second mutation discipline and deadlock surface for no gain.

Merge conflict window: while the worker runs, the only writers to the instance row are reload (preserves the fields in question) and dialogs, which are gated (D3). Status poller skips `Restarting`. So the merge is effectively last-writer-safe.

### D2: Enqueue-time snapshot decides the path

The keypress handler resolves, at enqueue time, whether this is a respawn (`tmux session exists`) or a recovery (recoverable: slots exist, session dead), mirroring today's `restart_action` + action-time re-checks. The worker re-validates cheaply (e.g. recovery re-checks `is_recoverable` against live tmux) because the queue introduces a delay between decision and execution; a no-longer-valid request completes as a no-op result rather than an error.

### D3: Gating is by `restart_in_flight`, checked at the input layer

At enqueue: `restart_in_flight = true`, `status = Restarting`. Cleared when the result is applied. While set, `input.rs` rejects for that instance: Enter/jump attach, `d`, `c`/`r`/`C`/`R`, `e` rename is allowed (metadata only, does not touch panes). The checks sit next to the existing `Status::Deleting` guards and follow the same shape.

Why not gate on `status == Restarting` alone: the synchronous `C`/`R` path also passes through `Restarting` transiently; `restart_in_flight` names the async case precisely and already exists.

### D4: Serial queue, one worker thread for the poller's lifetime

Identical lifecycle to `DeletionPoller` (spawn on construction, `mpsc` request/result channels, worker exits when the request sender drops). Serialization also sidesteps concurrent `Store::open_with_schema` writers from multiple restart threads.

### D5: Background tmux traffic during attach is acceptable

`auto_confirm_panes`'s doc comment warns a background thread "would stall once attach starts". The risk it names is keystroke injection racing the user inside the same session; D3's attach gate closes that for the restarting instance. For other sessions, background `capture-pane`/`send-keys` against instance A while the user is attached to instance B is ordinary concurrent tmux server traffic, and precedent exists (monitor-driven reconcile runs during attach). The worker only ever touches pane ids recorded for its own instance.

## Risks / Trade-offs

- [Worker dies mid-restart (panic)] → instance stuck in `Restarting`/`restart_in_flight` forever. Mitigation: worker wraps the pipeline in `catch_unwind` and always sends a result (error variant); the apply step always clears the flag.
- [User quits TUI while a restart is queued/running] → worker thread is detached like `DeletionPoller`'s; the tmux-side respawn either completed or it did not, and the next TUI start reads reality from tmux + store. The `restart_in_flight` flag is `#[serde(skip)]`, so no gating sticks. The `Restarting` status itself IS serialized: if an unrelated save fires during the window (rename, deletion result, creation), a later TUI process loads the row as `Restarting`, which the status poller never repolls (tier 0) -- the row shows a stale spinner until the user presses a restart key or attaches (neither is gated, since the flag deserializes false). Same exposure class as a persisted `Deleting` status today. Accept.
- [Second TUI process on the same profile restarts the same instance concurrently] → same exposure as today (sync restarts in two processes can already collide); serialization is per-process only. Accept, unchanged.
- [Queued request grows stale while waiting behind another restart] → `restart_in_flight` is set at enqueue time, so the D3 gate covers queued-but-not-started instances too (no delete/attach can slip in); worker-side re-validation (D2) catches tmux-side drift from outside this process.
- [Auto-confirm's 12s cap serializes badly: 3 queued restarts could take ~36s to all finish] → accepted trade-off per requirements; the list shows each as `Restarting` so progress is visible.

## Migration Plan

Pure in-process refactor, no stored-data changes, no migration. Rollback is reverting the commit.

## Open Questions

None blocking. If e2e later shows the `restart_in_flight` gate needs to surface feedback (e.g. a status-bar hint when a gated key is pressed), that is a follow-up UX nicety, not part of this change.
