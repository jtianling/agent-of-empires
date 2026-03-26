## Why

The notification monitor runs its own simplified `detect_live_status()` that bypasses the optimizations already implemented in the TUI status poller pipeline: activity gating, spike detection, acknowledged-waiting mapping, and capture cache. This causes status inconsistencies between the TUI and the notification bar, unnecessary subprocess overhead (per-session `list-panes` + `capture-pane` every 2 seconds), and false Running flickers from transient content spikes.

Agent-deck (Go counterpart) solves this with a unified detection pipeline shared between its TUI and notification bar, adaptive polling intervals, and global tmux option writes instead of per-session updates.

## What Changes

- Notification monitor reuses the existing detection infrastructure (activity gating, spike detection, capture cache, acknowledged mapping) instead of its own raw `detect_live_status()`
- Batch pane title queries replace per-session `list-panes` calls in the notification monitor
- Per-session `@aoe_waiting` option writes consolidated into a single global tmux option write per poll cycle
- Adaptive polling interval: 1s when any session is Running, 2s when any is Waiting, 3s when all Idle
- Notification bar key bindings: pressing assigned number keys (`Ctrl+b 1-6`) switches to that session and marks it acknowledged

## Capabilities

### New Capabilities
- `notification-keybindings`: Dynamic tmux key bindings that switch to notification bar sessions and mark them acknowledged

### Modified Capabilities
- `status-detection`: Notification monitor shares the same detection pipeline as TUI status poller instead of its own separate path
- `activity-gated-polling`: Activity gating applies to notification monitor, not just TUI poller
- `spike-detection`: Spike detection applies to notification monitor, not just instance update
- `acknowledged-waiting`: Notification monitor respects acknowledged state when determining notification visibility
- `capture-cache`: Notification monitor uses cached captures instead of raw subprocess calls
- `batch-pane-query`: Notification monitor uses the shared pane info cache for title detection

## Impact

- `src/tmux/notification_monitor.rs`: Major refactor - replace `detect_live_status()` with shared pipeline, add adaptive polling, consolidate option writes
- `src/tmux/status_bar.rs`: Global notification option instead of per-session `@aoe_waiting`
- `src/tmux/mod.rs`: Pane info cache and session cache may need to be refreshable from the notification monitor process (currently they run in the TUI process context)
- `src/session/instance.rs`: Spike detection and acknowledged state may need to be tracked in the notification monitor's process memory (it runs as a separate `aoe tmux monitor-notifications` process, not in the TUI)
- Key concern: the notification monitor is a **separate OS process** from the TUI. Shared caches (PaneInfoCache, CaptureCache) live in static variables in the TUI process. The monitor needs its own cache instances or a shared mechanism.
