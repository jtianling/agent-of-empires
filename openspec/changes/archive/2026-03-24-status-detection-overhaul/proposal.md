## Why

AoE's status detection works but is inefficient and imprecise compared to agent-deck's approach. Every poll cycle captures pane content for every instance regardless of whether anything changed, there is no fast path to avoid expensive `capture-pane` calls, and status flickers during tool transitions because there is no grace period. Batch queries, capture caching, and acknowledged/unacknowledged waiting distinction are all missing.

## What Changes

- Add tmux `window_activity` timestamp tracking to skip `capture-pane` when nothing changed (biggest perf win)
- Add pane title fast-path detection: check for Braille spinner chars in pane title before falling back to content capture
- Add spinner grace period (500ms) to prevent Running/Idle flicker during tool transitions
- Add spike detection (1s confirmation window) to prevent false busy from transient output
- Distinguish Waiting (user hasn't seen output) vs Idle (user acknowledged) with attach/reset lifecycle
- Batch `list-panes -a` query per poll cycle instead of per-instance pane queries
- Add capture result caching (500ms TTL) to avoid redundant `capture-pane` calls within a cycle

## Capabilities

### New Capabilities

- `activity-gated-polling`: Skip expensive pane capture when tmux window_activity timestamp hasn't changed since last check
- `title-fast-path`: Detect Running status from pane title spinner chars without capturing pane content
- `spinner-grace-period`: Hold Running status for 500ms after spinner disappears to prevent flicker during tool transitions
- `spike-detection`: Require activity confirmation within 1s window before declaring busy status
- `acknowledged-waiting`: Distinguish Waiting (needs attention) from Idle (user already saw output) via attach/detach lifecycle
- `batch-pane-query`: Single `list-panes -a` call per poll cycle replaces per-instance pane info queries
- `capture-cache`: Cache capture-pane results with 500ms TTL to avoid redundant subprocess calls

### Modified Capabilities

- `status-detection`: Detection pipeline gains fast-path layers (title check, activity gate) before content parsing; Waiting/Idle semantics refined with acknowledgment tracking

## Impact

- `src/tmux/mod.rs`: Session cache expanded to include pane info (title, command, activity timestamp, dead flag)
- `src/tmux/session.rs`: `detect_status()` gains title fast-path and capture caching
- `src/session/instance.rs`: `update_status()` gains activity gate, spike detection, grace period, acknowledged state
- `src/tui/status_poller.rs`: Batch pane query at start of each cycle, pass activity data to instances
- `src/tui/app.rs`: Set acknowledged flag on session attach
- `src/session/instance.rs`: New fields for `last_activity`, `last_spinner_seen`, `spike_start`, `acknowledged`
