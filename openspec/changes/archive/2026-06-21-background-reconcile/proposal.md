## Why

The reconciler that snapshots `pane_live` captures into the durable `agent_slot` table only runs on the TUI status-poller tick, which is driven solely from the home (session list) view. While the user is attached to any session, the TUI main loop is blocked on a synchronous `tmux attach-session` call, so the poller stops ticking and reconcile is suspended. Because AoE's normal usage is to stay attached inside an agent, `agent_slot` is almost never updated in practice -- even though capture into `pane_live` keeps working. `agent_slot` is the only data source for the `R` all-pane resume (w03) and cold-start recovery (w04), so those features effectively cannot get fresh data.

## What Changes

- The reconciler (`reconcile_all`) gains a second, attach-independent driver: it runs periodically inside the long-lived `aoe tmux monitor-notifications` background process, which is unaffected by TUI attach blocking.
- The reconcile pass in the notification monitor is throttled (a minimum interval, like the existing poller's `RECONCILE_INTERVAL`) so it does not over-query tmux on every short poll cycle.
- `reconcile_all` is reused unchanged; only a new call site is added. The existing home-view-driven reconcile is kept (reconcile is idempotent, so the two drivers do not conflict).
- Test coverage closes the gap that existing e2e tests miss: a new attach-scoped e2e asserts that a capture produced while attached is reflected in `agent_slot` within a bounded time without returning to the home view.
- Not a breaking change: no data/schema change, no migration. Pure change to when/where reconcile is driven.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities
- `pane-session-capture`: the "Reconciler snapshots pane captures into durable slots" requirement no longer ties reconcile exclusively to the status-poller tick. The reconciler SHALL also run from the long-lived notification-monitor process so that `agent_slot` keeps advancing while the TUI is attached to a session (i.e. when the status-poller is not ticking).

## Impact

- `src/tmux/notification_monitor.rs`: add a throttled reconcile call inside `run_notification_monitor`'s loop (instances are already loaded there via `Storage::load_with_groups`).
- `src/db/reconcile.rs`: consumed unchanged (`reconcile_all(profile, &instances)`).
- `tests/e2e/`: add an attach-scoped reconcile test (RED before the fix); keep the existing 26 agent-session e2e cases green as regression.
- No migration, no schema change, no change to the capture path (`hooks` -> `__record-pane` -> `pane_live`).
