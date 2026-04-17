## MODIFIED Requirements

### Requirement: Status detection pipeline order
The status detection pipeline in `update_status()` SHALL follow this layered order:

1. Skip if Stopped/Restarting/Deleting
2. Error cooldown check (30s)
3. Starting grace period (3s)
4. Session existence check
5. Hook-based detection (Claude/Cursor) -- apply acknowledged mapping; **only short-circuit when the hook status file is fresh (see "Hook status freshness check")**
6. Title fast-path (spinner in pane title from batch cache)
7. Activity gate (skip capture if window_activity unchanged)
8. Content-based detection via `capture-pane` + tool-specific patterns
9. Spike detection (1s confirmation for content-based Running)
10. Spinner grace period (500ms hold for Running-to-non-Running)
11. Acknowledged waiting mapping (Waiting + acknowledged -> Idle)
12. Shell/dead pane heuristics (existing behavior)

#### Scenario: Full pipeline execution order
- **WHEN** `update_status()` is called for a non-hook agent with changed activity
- **THEN** the detection SHALL proceed through layers 1-12 in order
- **AND** each layer that produces a definitive result SHALL short-circuit subsequent layers

#### Scenario: Fresh hook agent skips layers 6-10
- **WHEN** `update_status()` is called for a hook-based agent (Claude/Cursor)
- **AND** the hook status file exists
- **AND** the hook status file is fresh (mtime within the freshness window)
- **THEN** the detection SHALL use the hook result directly
- **AND** apply only the acknowledged mapping (layer 11) and pane-dead override
- **AND** skip title fast-path, activity gate, content detection, spike detection, and grace period

#### Scenario: Stale hook agent falls through to content detection
- **WHEN** `update_status()` is called for a hook-based agent (Claude/Cursor)
- **AND** the hook status file exists but its mtime is older than the freshness window
- **THEN** the detection SHALL NOT short-circuit on the hook result
- **AND** SHALL proceed through the non-hook detection path (layers 6-10)
- **AND** the final status SHALL come from content-based detection (plus spike/grace/acknowledged mapping)

#### Scenario: Missing hook file falls through to content detection
- **WHEN** `update_status()` is called for a hook-based agent (Claude/Cursor)
- **AND** the hook status file does not exist
- **THEN** the detection SHALL proceed through the non-hook detection path (layers 6-10)

#### Scenario: Title fast-path short-circuits content detection
- **WHEN** the pane title contains a spinner character
- **THEN** the detection SHALL return Running
- **AND** skip activity gate, content detection, and spike detection
- **AND** update `last_spinner_seen` for grace period tracking

## ADDED Requirements

### Requirement: Hook status freshness check
Hook-based status SHALL be trusted for short-circuiting only when the hook status file has been written recently. The "freshness window" is the maximum age (measured from the file's mtime to the current time) within which the hook result is considered authoritative. When the file is older than the freshness window, it is "stale" and MUST be treated as absent for the purpose of status detection.

The freshness window SHALL be a module-level constant in the hooks module. It SHALL be at least 30 seconds to tolerate long agent turns without spurious fallback, and SHALL NOT exceed 5 minutes.

#### Scenario: Fresh hook file is authoritative
- **WHEN** the hook status file mtime is within the freshness window of the current time
- **THEN** `read_hook_status()` callers SHALL treat the returned status as authoritative

#### Scenario: Stale hook file is ignored
- **WHEN** the hook status file mtime is older than the freshness window
- **THEN** status detection SHALL behave as if the hook file did not exist
- **AND** SHALL fall through to content-based detection
- **AND** SHALL NOT modify or delete the hook file on disk

#### Scenario: Hook reader exposes mtime
- **WHEN** a caller reads hook status
- **THEN** the hook module SHALL expose both the status value and the file's mtime (or a derived fresh/stale flag) so the caller can apply freshness gating

### Requirement: Notification monitor applies hook freshness gating
The notification monitor SHALL apply the same hook freshness check as the TUI status poller. A stale hook file SHALL NOT keep the monitor's view of a session pinned to a past status; instead the monitor SHALL fall through to content-based detection via its existing shared pipeline.

#### Scenario: Monitor falls through on stale hook
- **WHEN** the notification monitor checks a session's hook status
- **AND** the hook file is stale (mtime older than the freshness window)
- **THEN** the monitor SHALL proceed to title fast-path and content-based detection
- **AND** SHALL NOT report the stale hook value to consumers
