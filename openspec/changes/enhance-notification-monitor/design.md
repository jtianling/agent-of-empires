## Context

The notification monitor is a background process (`aoe tmux monitor-notifications`) that polls all AoE sessions every 2 seconds, detects their live status, and writes per-session `@aoe_waiting` tmux options for the status bar to display. It currently uses its own simplified `detect_live_status()` function that spawns per-session subprocesses without leveraging the optimizations already built into the TUI status poller: activity gating, spike detection, acknowledged-waiting mapping, capture caching, and batch pane queries.

Key architectural constraint: the notification monitor runs as a **separate OS process** from the TUI. Static caches (`PaneInfoCache`, `SessionCache`, `CaptureCache`) are per-process, so the monitor gets its own cache instances. Cross-cycle state (spike timers, activity timestamps, acknowledged flags) must be maintained in the monitor's process memory since it loads fresh instances from `sessions.json` each cycle.

Agent-deck (Go counterpart) solves similar problems with: a single global `status-left` write per cycle, adaptive polling intervals, control mode pipes for zero-subprocess monitoring, and activity-based polling skips.

## Goals / Non-Goals

**Goals:**
- Notification monitor uses the same detection quality as TUI (activity gating, spike detection, capture cache)
- Reduce subprocess overhead from ~3N per cycle to ~3 (two batch queries + only changed sessions get capture-pane)
- Consistent status between TUI display and notification bar
- Acknowledged state respected: sessions the user has already viewed show as Idle, not Waiting
- Adaptive polling: faster when sessions are active, slower when all idle
- Notification bar key bindings for quick session switching

**Non-Goals:**
- Tmux control mode pipes (major complexity, save for later)
- Transition notification daemon (sending messages to parent sessions)
- Multiple display modes (minimal/show-all) -- current mode works fine
- Changing the notification monitor from a separate process to a TUI thread

## Decisions

### 1. Monitor maintains its own per-session state map

The monitor currently loads fresh `Instance` structs from `sessions.json` each cycle, losing all cross-cycle state. Instead, it will maintain a `HashMap<String, MonitorSessionState>` in process memory:

```rust
struct MonitorSessionState {
    last_window_activity: i64,
    last_full_check: Instant,
    last_status: Status,
    spike_start: Option<Instant>,
    pre_spike_status: Option<Status>,
}
```

**Why**: Instance structs from storage don't carry transient state. The TUI poller keeps this state on its own copies. The monitor needs its own.

**Alternative considered**: Share state via a file or tmux options. Rejected because it adds I/O overhead and complexity for data that only the monitor needs.

### 2. Use batch queries at cycle start

Each poll cycle begins with two batch subprocess calls:
1. `tmux list-sessions -F ...` (session activity timestamps)
2. `tmux list-panes -a -F ...` (pane titles, dead state, PIDs)

These populate the existing `SessionCache` and `PaneInfoCache` statics in the monitor process. Individual `list-panes -t <session>` calls in `get_pane_title()` are replaced with cache lookups.

**Why**: Reduces per-session subprocess spawning from O(N) to O(1).

### 3. Activity gating skips unchanged sessions

After refreshing the session cache, compare each session's `window_activity` with the stored value in `MonitorSessionState`. If unchanged and last full check was < 10 seconds ago, skip capture-pane and reuse the previous status.

**Why**: Most sessions are idle most of the time. Skipping capture-pane for them saves the most expensive operation.

**Exception**: Hook-based agents (Claude, Cursor) always read their hook file regardless of activity, since that's cheap.

### 4. Global notification option instead of per-session writes

Replace N `set-option -t <session> @aoe_waiting` calls with a single `set-option -g @aoe_notification_bar` call containing the full notification text. Each session's `status-left` format references this global option.

**Why**: Agent-deck uses this approach. Reduces tmux option writes from O(N) to O(1). The tradeoff is that the notification bar shows the same content in all sessions (excluding the current session filtering), which can be done in the status-left format string using `#{?#{==:#{session_name},aoe_xxx},...}` conditionals - but this gets complex. A simpler approach: keep per-session writes for now but batch them into a single tmux command using `\;` separators.

**Decision**: Keep per-session `@aoe_waiting` for now (simpler, already working), but batch the writes into a single `tmux` invocation with `\;` separators to reduce subprocess count from N to 1.

### 5. Adaptive polling interval

The monitor adjusts its sleep duration based on detected states:
- Any session Running: 1 second (fast tracking)
- Any session Waiting (no Running): 2 seconds (current default)
- All sessions Idle: 3 seconds (save resources)

**Why**: Agent-deck uses this. Running sessions benefit from faster status updates. Idle sessions don't need frequent polling.

### 6. Spike detection in monitor

Apply the same spike detection logic from `Instance::apply_spike_detection()`: when content-based detection first shows Running after a non-Running state, hold the previous status for one cycle. Commit to Running only after confirmation in the next cycle.

**Why**: Prevents false Running flashes in the notification bar from cursor blink or terminal redraws. Currently only the TUI has this.

### 7. Notification bar key bindings

When the notification bar displays entries like `[1] ◐ session-title`, bind tmux keys `1` through `6` (under the AoE prefix key) to switch to the corresponding session. On switch, write the session ID to an ack signal file that the monitor reads on the next cycle to mark as acknowledged.

**Why**: Agent-deck has this. Lets users quickly jump to waiting sessions without returning to the TUI.

**Mechanism**:
1. Monitor writes `bind-key -T aoe_notify 1 run-shell "echo <session_id> > /tmp/aoe-ack-signal && tmux switch-client -t <session_name>"`
2. Monitor checks `/tmp/aoe-ack-signal` at cycle start, marks the session acknowledged
3. Bindings are cleaned up when the monitor exits

### 8. Capture cache for monitor

The monitor uses `capture_pane_cached()` (500ms TTL) instead of direct `Command::new("tmux").args(["capture-pane", ...])` calls. Since the monitor is a separate process, it gets its own cache instance. Within a single poll cycle, if multiple consumers need the same pane content, the cache prevents duplicate captures.

**Why**: Consistency with TUI approach, and protects against future additions to the monitor that might need the same capture.

## Risks / Trade-offs

**[Risk] Monitor state map grows with sessions** -> State is small (few fields per session) and capped by session count. Stale entries are cleaned up when sessions disappear from `list_aoe_sessions()`.

**[Risk] Ack signal file race condition** -> The monitor is the only reader; it reads and deletes atomically. Multiple key presses between cycles overwrite the file, but only the last matters (user is looking at that session now).

**[Risk] Batched tmux option writes may fail partially** -> If the batched `tmux` command fails, fall back to per-session writes. Log the error for debugging.

**[Risk] Adaptive polling adds complexity** -> The implementation is a simple match on detected states. If it causes issues, easy to revert to fixed 2s.
