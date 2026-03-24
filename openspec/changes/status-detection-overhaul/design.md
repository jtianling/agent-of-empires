## Context

AoE's status detection currently follows a straightforward pipeline: the background `StatusPoller` thread sends instance lists to a polling loop, which calls `Instance::update_status()` for each. That method checks hooks first (Claude/Cursor), then falls back to `session.detect_status(tool)` which calls `capture_pane(50)` and runs the tool-specific pattern matcher.

The existing session cache (`tmux/mod.rs`) stores `session_name -> session_activity` from `list-sessions` and is refreshed once per poll cycle. However, pane-level info (title, current command, dead flag) is queried individually per instance via separate `tmux display-message` calls.

Reference project agent-deck demonstrates several optimizations that reduce tmux subprocess calls and improve detection accuracy.

## Goals / Non-Goals

**Goals:**

- Reduce tmux subprocess calls per poll cycle (currently O(n) capture-pane + O(n) pane-info queries)
- Eliminate unnecessary `capture-pane` calls when nothing changed in a session
- Prevent status flicker during tool transitions (spinner disappears briefly between tool calls)
- Prevent false Running detection from transient output
- Distinguish "needs user attention" (Waiting) from "user is looking at it" (Idle after attach)
- Keep all optimizations backward-compatible with existing status semantics

**Non-Goals:**

- Changing the hook-based detection path (Claude/Cursor) -- it already works well
- Adding whimsical word detection or other agent-deck-specific patterns
- Changing the Status enum variants (Running/Waiting/Idle/Error etc. stay the same)
- Real-time event-driven detection (we keep the polling model)

## Decisions

### D1: Activity-gated polling via window_activity timestamp

**Decision**: Expand the session cache to include `window_activity` timestamps. Before calling `capture_pane`, compare the current `window_activity` against the value from the last poll. Skip capture if unchanged and more than 2s since last full check.

**Why over alternatives**: tmux already tracks `window_activity` -- we just need to read it. This avoids adding file watchers or inotify complexity. The 2s floor ensures we don't miss rapid state changes.

**Implementation**: Extend `refresh_session_cache()` to query `#{window_activity}` alongside `#{session_name}`. Store per-session last-checked activity in the `StatusPoller` state. Pass activity timestamps to `update_status()` as a parameter.

### D2: Batch pane info query

**Decision**: Replace per-instance `display-message` calls (for pane dead, pane command, pane title) with a single `list-panes -a -F "#{session_name}\t#{pane_title}\t#{pane_current_command}\t#{pane_dead}\t#{pane_pid}"` call per poll cycle.

**Why over alternatives**: One subprocess call replaces N calls. agent-deck uses this approach with a 4s cache. We can use the same poll-cycle scope (no separate TTL needed since we already refresh once per cycle).

**Implementation**: Add a `PaneInfoCache` struct in `tmux/mod.rs` alongside the existing `SessionCache`. Populate it in `refresh_session_cache()` (renamed to `refresh_tmux_cache()`). Provide `get_cached_pane_info(session_name) -> Option<PaneInfo>` for consumers.

### D3: Pane title fast-path for spinner detection

**Decision**: Before falling back to `capture_pane`, check the pane title (from the batch cache) for Braille spinner characters. If any spinner char is found in the title, return `Running` immediately.

**Why over alternatives**: Pane title is already available from the batch query (D2) -- zero extra cost. Many agents (Claude, OpenCode) put spinner chars in the pane title while working. This skips the expensive content capture entirely for the most common case (agent actively running).

**Implementation**: Add `detect_status_from_title(title: &str) -> Option<Status>` in `status_detection.rs`. Call it in `update_status()` before the content-based path.

### D4: Spinner grace period (500ms)

**Decision**: When a session transitions from Running (spinner visible) to non-Running, hold the Running status for 500ms before switching. If a spinner reappears within that window, the status never flickered.

**Why over alternatives**: Simple timestamp-based approach. No need for complex state machines. 500ms matches agent-deck's proven value and is imperceptible to users.

**Implementation**: Add `last_spinner_seen: Option<Instant>` to the instance's transient state (not serialized). In `update_status()`, if previous status was Running and new detection is not Running, check if `last_spinner_seen` was within 500ms -- if so, keep Running.

### D5: Spike detection (1s confirmation window)

**Decision**: When transitioning from Idle/Waiting to Running, require the Running signal to persist for at least one more poll cycle (up to 1s) before committing to Running status. Show the previous status during the confirmation window.

**Why over alternatives**: Prevents single-frame transient output (e.g., a brief spinner flash in scrollback) from causing a false Running detection. The 1s window is short enough to not noticeably delay real Running detection.

**Implementation**: Add `spike_start: Option<Instant>` to transient instance state. On first Running detection after non-Running, set `spike_start = now()` and keep the old status. On next poll, if still Running and spike_start > 1s ago, commit to Running and clear spike_start.

### D6: Acknowledged waiting distinction

**Decision**: Add an `acknowledged: bool` field to Instance (transient, not serialized). When the user attaches to a session, set `acknowledged = true`. When new activity is detected (window_activity changes), reset to `false`. Map: if content detection returns Waiting and `acknowledged == true`, report Idle instead.

**Why over alternatives**: This matches agent-deck's Waiting/Idle semantics without changing the Status enum. The distinction is purely in how Waiting maps to the final status -- "needs attention" vs "user already saw it".

**Implementation**: Set acknowledged in `app.rs` attach flow. Reset in `update_status()` when activity changes. The mapping is a simple conditional in `update_status()`.

### D7: Capture result caching (500ms TTL)

**Decision**: Cache the result of `capture_pane()` per session with a 500ms TTL. Multiple consumers (status detection, resume token extraction) within the same poll cycle reuse the cached content.

**Why over alternatives**: Currently resume token extraction calls `capture_pane(100)` separately after status detection already called `capture_pane(50)`. Caching eliminates this redundancy. 500ms TTL ensures freshness while covering the full poll cycle.

**Implementation**: Add a `CaptureCache` in `tmux/session.rs` or as part of the `PaneInfoCache`. Key by session name, store `(content, Instant)`. Return cached content if within TTL.

## Risks / Trade-offs

- **[Spike detection delays real Running detection by ~1s]** -> Acceptable because the first poll shows Starting/previous status, and the second poll (within 1s) confirms. Users won't notice the delay.
- **[Grace period may briefly show Running when agent is actually done]** -> 500ms is imperceptible. Better than flickering.
- **[Batch pane query returns all panes, not just AoE sessions]** -> Filter by `aoe_` prefix. Non-AoE panes are ignored. Memory overhead is negligible.
- **[Acknowledged flag lost on TUI restart]** -> Acceptable. On restart, all sessions start unacknowledged (Waiting if at prompt), which is the safer default.
- **[Activity gate might miss changes if tmux doesn't update window_activity]** -> The 2s floor ensures periodic full checks. Hook-based agents (Claude/Cursor) bypass this entirely.
