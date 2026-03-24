## MODIFIED Requirements

### Requirement: Status detection pipeline order
The status detection pipeline in `update_status()` SHALL follow this layered order:

1. Skip if Stopped/Restarting/Deleting
2. Error cooldown check (30s)
3. Starting grace period (3s)
4. Session existence check
5. Hook-based detection (Claude/Cursor) -- apply acknowledged mapping
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

#### Scenario: Hook agent skips layers 6-10
- **WHEN** `update_status()` is called for a hook-based agent (Claude/Cursor)
- **AND** the hook status file exists and is fresh
- **THEN** the detection SHALL use the hook result directly
- **AND** apply only the acknowledged mapping (layer 11) and pane-dead override
- **AND** skip title fast-path, activity gate, content detection, spike detection, and grace period

#### Scenario: Title fast-path short-circuits content detection
- **WHEN** the pane title contains a spinner character
- **THEN** the detection SHALL return Running
- **AND** skip activity gate, content detection, and spike detection
- **AND** update `last_spinner_seen` for grace period tracking
