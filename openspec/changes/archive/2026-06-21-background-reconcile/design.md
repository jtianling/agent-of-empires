## Context

The agent-session capture chain has two layers:
1. **Capture** -- the status hook shells out to `aoe __record-pane`, which writes `pane_live` directly to SQLite. This runs inside the agent process, independent of the TUI, and works correctly.
2. **Reconcile** -- `reconcile_all` (`src/db/reconcile.rs:153`) snapshots `pane_live` into the durable `agent_slot` table (the only data source for w03 `R` all-pane resume and w04 cold-start recovery).

Today reconcile is driven only from the TUI status poller (`src/tui/status_poller.rs:111`), which is fed by `home/mod.rs:406 request_status_refresh`, called from the `app.rs:266` main event loop. When the user attaches to a session, `app.rs:810 with_raw_mode_disabled(terminal, || tmux_session.attach())` blocks the main loop synchronously until detach. The status poller's channel `recv()` therefore gets no requests, so reconcile is suspended for the entire attach. Because AoE is normally used while attached inside an agent, `agent_slot` is chronically stale/empty even though `pane_live` keeps updating.

The long-lived `aoe tmux monitor-notifications` process (`src/tmux/notification_monitor.rs:668 run_notification_monitor`) already runs an independent poll loop (1-3s cadence), is spawned by `ensure_notification_monitor` at TUI startup, is detached (own process, stdin/out/err null), and is therefore unaffected by TUI attach blocking. Its loop already loads instances via `Storage::load_with_groups` (inside `update_notification_options`).

## Goals / Non-Goals

**Goals:**
- `agent_slot` keeps advancing while the user is attached to a session (i.e. when the status poller is not ticking).
- Reuse `reconcile_all` unchanged; add only a new, attach-independent driver.
- Do not regress the existing home-view-driven reconcile or the 26 agent-session e2e cases.
- Close the test coverage gap with an attach-scoped reconcile e2e.

**Non-Goals:**
- Rewriting reconcile logic, slot assignment, or the capture path (`hook -> __record-pane -> pane_live`).
- Implementing w04 cold-start recovery itself (this change only makes its data source fresh).
- Fixing the pre-existing spec/impl divergence where `agent-session-store` / `pane-session-capture` specs describe an `agent_slot.tmux_pane` column that the current table does not have. Out of scope; noted under Risks.
- Removing the home-view reconcile driver.

## Decisions

### Decision 1: Drive reconcile from the notification-monitor loop (Plan B)
Add a throttled `reconcile_all(profile, &instances)` call inside `run_notification_monitor`'s loop.

Rationale:
- That process is already long-lived, independent of the TUI loop, and unaffected by attach blocking -- exactly the property the home-view driver lacks.
- It already loads instances each cycle, so no new data plumbing is required.
- Its lifecycle is tied to session existence (it breaks when no aoe sessions remain), which matches when reconcile is needed.

Alternatives considered:
- **Plan A (status_poller self-timer):** make the poller tick on its own timer instead of waiting for `request_refresh`. Rejected: the poller still lives in the TUI process whose loop is blocked during attach; it would need its own thread plus an independent instances source, duplicating what the monitor already has.
- **Plan C (new dedicated daemon):** a separate reconcile process. Rejected: a second always-on process to manage when an existing one already fits.

### Decision 2: Keep the home-view driver too (dual, idempotent)
`reconcile_all` is idempotent (upsert by `(instance_id, slot)`), so running it from both the home-view poller and the monitor is safe. Keeping both avoids touching `status_poller` / `home` and preserves current behavior when the user is on the home view.

### Decision 3: Throttle the monitor's reconcile independently
The monitor polls every 1-3s; reconcile enumerates panes via `tmux list-panes` per session. Gate reconcile behind a minimum interval (mirroring `status_poller`'s `RECONCILE_INTERVAL`, 750ms) tracked with a `last_reconcile: Instant` local in the loop, so a fast 1s poll cadence does not multiply tmux queries unnecessarily. The monitor cadence (>=1s) already keeps reconcile within ~1s of a capture, which is the freshness target.

### Decision 4: Instances source inside the monitor loop
`run_notification_monitor` already computes `session_names` and calls `update_notification_options`, which loads `(instances, groups)`. Load instances for reconcile via the same `Storage::new(profile).load_with_groups()` (cheap, file-backed) in the loop, guarded by the throttle, and pass to `reconcile_all`. Avoids changing `update_notification_options`' return contract.

## Risks / Trade-offs

- **Concurrent writers (monitor + home-view poller both reconcile the same `aoe.db`)** -> The store opens in WAL mode, which already tolerates concurrent hook-subprocess writers plus the reconciler (`src/db/mod.rs:239`). `reconcile_all` is idempotent, so interleaved passes converge. Mitigation: rely on existing WAL + idempotency; no new locking.
- **Monitor not running (no aoe sessions)** -> reconcile won't run, but there are no sessions to reconcile then. Acceptable; the home-view driver still covers the on-list case.
- **Extra `tmux list-panes` load from a second driver** -> bounded by Decision 3's throttle; the monitor already runs `refresh_session_cache` / `refresh_pane_info_cache` each cycle.
- **Pre-existing spec/impl divergence (`agent_slot.tmux_pane` in spec, absent in table)** -> not introduced or fixed here; flagged so a future change can reconcile spec and schema.

## Migration Plan

No data migration and no schema change -- this is purely a change to when/where `reconcile_all` is driven. Rollback is a straight revert of the new call site; no stored state is affected.
