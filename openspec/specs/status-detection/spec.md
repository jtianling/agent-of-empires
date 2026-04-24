# Capability Spec: Status Detection

**Capability**: `status-detection`
**Created**: 2026-03-06
**Status**: Stable

## Overview

Status detection polls the tmux pane content of each agent session to determine its current
state (Running, Waiting, Idle, Error, etc.). Each agent has a dedicated detection function
that recognizes the agent's specific UI patterns in the terminal output.

## Detection Architecture

```
TUI Status Poller (background task)
    │
    ▼  every N seconds
Instance::update_status()
    │
    ├── session.exists()? ──No──▶ Status::Error
    │
    ▼
session.detect_status(tool_name)
    │
    ├── session.capture_pane()  ← raw tmux pane content
    │
    ▼
AGENTS[tool].detect_status(pane_content)
    │
    ▼
Status: Running | Waiting | Idle | Unknown
```

## Per-Agent Detection Patterns

Each detection function receives **raw, non-lowercased** pane content and returns a `Status`.

### Claude

Detects by looking for specific UI strings in pane output:
- `Running`: actively generating (tool use, streaming)
- `Waiting`: permission prompt visible
- `Idle`: ready for next message
- Falls back to `Idle` if no recognized pattern

### OpenCode

Detects its specific TUI state indicators.

### Vibe (Mistral Vibe)

Detects Vibe-specific patterns.

### Codex

Codex CLI uses a Rust-based ink TUI. Detection patterns are based on its actual terminal output:
- `Running`: `esc to interrupt` in last lines, or bullet spinner `\u{2022}` / `\u{25e6}` at line start
- `Waiting` (approval): `Press enter to confirm`, or `\u{203a}` (single right-pointing angle) followed by numbered options (`1.`, `2.`, `3.`)
- `Waiting` (input): `\u{203a}` prompt character at the start of a line in the last 5 lines
- `Idle`: none of the above match

Note: Codex uses `\u{203a}` (not ASCII `>`) as its prompt, and `\u{2022}`/`\u{25e6}` (not braille) as its spinner.

### Gemini

Detects Gemini CLI patterns.

### Cursor

Detects Cursor agent CLI patterns.

## Status Transition Rules

| Condition | Resulting Status |
|-----------|-----------------|
| tmux session does not exist | `Error` |
| pane is dead (process exited) | `Error` |
| within 3s of `start()` | `Starting` (grace period) |
| already `Error`, within 30s of last check | `Error` (cached, no re-check) |
| `Idle` + custom command + pane alive | `Unknown` (agent-specific idle not applicable) |
| `Idle` + custom command + pane dead | `Error` |
| any other detected state | use detected state directly |

## Polling

Status polling runs as a background task in the TUI. The interval is configurable.
The TUI re-renders when status changes are detected.

For CLI `status` command, status is read from persisted storage (not live-polled).

## Managed Pane Titles

For agents that do not set their own terminal title via OSC 0 (`sets_own_title: false` in agent registry), the status poller actively manages the tmux pane title using `select-pane -T`:

- `Status::Waiting` -> pane title set to `\u{270b} <session title>` (hand icon prefix)
- Any other status -> pane title set to `<session title>` (plain)

Title updates are deduplicated (only written when the desired title differs from the last set value) to avoid unnecessary tmux calls. An initial pane title is also set at session creation time via `apply_all_tmux_options`.

Agents with `sets_own_title: true` (claude, gemini) set their own pane title and are not managed by this mechanism.

## Error Caching

To avoid expensive repeated tmux calls for sessions in error state, `Error` status is
cached for 30 seconds after detection (`last_error_check` timestamp). This prevents
thrashing when many sessions are errored simultaneously.

## Starting Grace Period

After `Instance::start()` is called, the session is held in `Starting` status for 3 seconds.
This prevents premature `Error` detection during the brief window between tmux session
creation and the agent process becoming visible in the pane.

## Requirements

### Requirement: Status poller captures resume token on pane death transition
When the status poller detects that an agent pane has transitioned from a non-error status to dead (Error), it SHALL attempt to extract a resume token from the pane output using the agent's configured `resume_pattern`. The extracted token SHALL be included in the `StatusUpdate` message sent back to the TUI.

#### Scenario: Resume token captured on first pane death detection
- **WHEN** the status poller polls an instance whose previous status was not `Error`
- **AND** the current poll detects the pane is dead (status transitions to `Error`)
- **AND** the agent has a `ResumeConfig` with a `resume_pattern`
- **THEN** the poller SHALL capture pane output and extract the resume token
- **AND** include the token in the `StatusUpdate` for that instance

#### Scenario: No token captured for agent without ResumeConfig
- **WHEN** the status poller detects a pane death transition
- **AND** the agent has no `ResumeConfig`
- **THEN** the `StatusUpdate` SHALL have `resume_token` set to `None`

#### Scenario: No token captured on subsequent polls of dead pane
- **WHEN** the status poller polls an instance whose previous status was already `Error`
- **AND** the pane is still dead
- **THEN** the poller SHALL NOT attempt to extract a resume token
- **AND** `resume_token` in the `StatusUpdate` SHALL be `None`

#### Scenario: Invalid token extracted is discarded
- **WHEN** the poller extracts a resume token that fails validation (non-hex/dash characters)
- **THEN** the `StatusUpdate` SHALL have `resume_token` set to `None`

### Requirement: StatusUpdate includes optional resume token field
The `StatusUpdate` struct SHALL include a `resume_token: Option<String>` field to carry captured resume tokens from the background poller thread to the TUI event loop.

#### Scenario: TUI applies resume token from status update
- **WHEN** the TUI receives a `StatusUpdate` with a non-None `resume_token`
- **THEN** it SHALL store the token on the corresponding Instance's `resume_token` field
- **AND** trigger a session save to persist the token

#### Scenario: StatusUpdate without token does not overwrite existing stored token
- **WHEN** the TUI receives a `StatusUpdate` with `resume_token` set to `None`
- **AND** the Instance already has a stored `resume_token`
- **THEN** the existing stored token SHALL NOT be overwritten

### Requirement: Status poller tracks previous status for transition detection
The status poller SHALL maintain a map of previous statuses per instance to distinguish first-time pane death (transition) from ongoing dead state (already known).

#### Scenario: Status map updated after each poll
- **WHEN** the status poller completes a poll cycle for an instance
- **THEN** it SHALL record the detected status in its previous-status map
- **AND** use this map on the next poll to determine if a transition occurred

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

### Requirement: Notification monitor uses shared detection pipeline
The notification monitor SHALL use the same three-tier detection pipeline as the TUI status poller: hook-based status, pane title fast-path (from batch pane info cache), and content-based detection (via capture cache). The monitor SHALL NOT use its own separate `detect_live_status()` function with direct subprocess calls.

#### Scenario: Monitor detects status via shared pipeline
- **WHEN** the notification monitor polls a session's status
- **THEN** it SHALL first check hook-based status via `read_hook_status()`
- **AND** then check pane title from the `PaneInfoCache` (no per-session subprocess)
- **AND** then fall back to `capture_pane_cached()` for content-based detection
- **AND** SHALL NOT spawn individual `tmux list-panes -t <session>` or `tmux capture-pane -t <session>` subprocesses

#### Scenario: Monitor maintains per-session detection state
- **WHEN** the notification monitor runs across multiple poll cycles
- **THEN** it SHALL maintain a `MonitorSessionState` map in process memory
- **AND** track `last_status`, `last_window_activity`, `last_full_check`, and spike detection fields per session
- **AND** this state SHALL persist across poll cycles within the monitor's process lifetime

#### Scenario: Stale session state cleaned up
- **WHEN** a session that was previously tracked no longer appears in `list_aoe_sessions()`
- **THEN** the monitor SHALL remove its `MonitorSessionState` entry

### Requirement: Adaptive polling interval
The notification monitor SHALL adjust its poll interval based on the aggregate state of all monitored sessions.

#### Scenario: Any session Running uses fast interval
- **WHEN** at least one session has status Running after detection
- **THEN** the monitor SHALL sleep for 1 second before the next cycle

#### Scenario: Any session Waiting uses medium interval
- **WHEN** no session is Running
- **AND** at least one session has status Waiting
- **THEN** the monitor SHALL sleep for 2 seconds before the next cycle

#### Scenario: All sessions Idle uses slow interval
- **WHEN** all sessions are Idle (or Error/Stopped)
- **THEN** the monitor SHALL sleep for 3 seconds before the next cycle

### Requirement: Batched tmux option writes
The notification monitor SHALL write all per-session `@aoe_waiting` options in a single tmux invocation using `\;` command separators.

#### Scenario: Multiple sessions updated in one call
- **WHEN** the monitor has computed notification text for N sessions
- **THEN** it SHALL execute a single `tmux` command with N `set-option` subcommands joined by `\;`
- **AND** SHALL NOT spawn N separate `tmux set-option` subprocesses

#### Scenario: Batched write failure falls back to individual writes
- **WHEN** the batched tmux command fails
- **THEN** the monitor SHALL fall back to individual `set-option` calls per session

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

### Requirement: Shell session primary-pane agent discovery on detach

When the user detaches from a tmux session whose configured tool is `shell`, AoE SHALL run exactly one agent-type discovery pass against the session's primary pane and cache the result in an in-memory field on the session instance.

The discovery SHALL reuse the existing `detect_agent_type_from_pane` helper. The returned value SHALL be stored in a non-persistent field (e.g. `detected_inner_agent: Option<String>`) on the in-memory session instance. The session's persisted `tool` field MUST NOT be modified.

If `detect_agent_type_from_pane` returns `Some("shell")` or `None`, the field SHALL be set to `None`. Otherwise it SHALL be set to `Some(<agent-name>)`.

#### Scenario: Agent detected on detach

- **WHEN** the user detaches from a shell session back to the AoE TUI and the primary pane's foreground process is a known agent (e.g. `claude`, `codex`, `gemini`)
- **THEN** the session's `detected_inner_agent` field is set to `Some("<agent-name>")` and the session's persisted `tool` remains `"shell"`

#### Scenario: No agent detected on detach

- **WHEN** the user detaches from a shell session back to the AoE TUI and the primary pane's foreground process is a bare shell or cannot be identified
- **THEN** the session's `detected_inner_agent` field is set to `None`

#### Scenario: Detection swap on subsequent detach

- **WHEN** a shell session has `detected_inner_agent = Some("claude")` from a previous detach, the user re-attaches, exits claude, starts codex, and detaches
- **THEN** on the new detach the field is overwritten to `Some("codex")`

#### Scenario: Non-shell session untouched

- **WHEN** the user detaches from a session whose tool is not `shell` (e.g. `claude`)
- **THEN** no shell-pane detection runs and no `detected_inner_agent` field is written for that session

### Requirement: Status polling uses detected inner agent for shell sessions

During status polling, when a session's tool is `shell` AND its in-memory `detected_inner_agent` is `Some(X)`, the primary-pane status detection SHALL dispatch to agent `X`'s content-based detector instead of the shell tool's stub detector.

When `detected_inner_agent` is `None`, primary-pane status detection SHALL fall back to the existing shell behavior (stub detector returning `Idle`, subsequently mapped to `Unknown` by the shell/custom-command heuristic).

When `detected_inner_agent` is `Some(X)` and agent `X`'s content-based detector returns a concrete status (`Running`, `Waiting`, `Idle`, `Error`), that status SHALL NOT be rewritten to `Unknown` by the `Idle → Unknown` heuristic that currently applies to shell sessions.

Status polling MUST NOT mutate `detected_inner_agent`. The field is only written on detach.

#### Scenario: Running status surfaces for detected agent

- **WHEN** a shell session has `detected_inner_agent = Some("claude")` and the primary pane content matches the claude "running" pattern
- **THEN** the TUI displays the session's status as `Running`

#### Scenario: Idle status surfaces for detected agent without override

- **WHEN** a shell session has `detected_inner_agent = Some("claude")` and claude's content detector returns `Idle`
- **THEN** the TUI displays the session's status as `Idle` (not `Unknown`)

#### Scenario: Shell fallback when no inner agent detected

- **WHEN** a shell session has `detected_inner_agent = None`
- **THEN** status polling uses the shell stub detector and the TUI displays `Unknown` (`?`) as it does today

#### Scenario: Detected agent with hook-only detector falls back to Unknown

- **WHEN** a shell session has `detected_inner_agent = Some(X)` where `X` has no usable content-based detector
- **THEN** the TUI displays `Unknown` (`?`) rather than a false signal

#### Scenario: Status poller does not clear detected field

- **WHEN** status polling runs on a shell session while the user is not attached
- **THEN** the session's `detected_inner_agent` field is not modified, regardless of the detected status value

## Functional Requirements

- **FR-001**: Detection functions MUST accept raw (non-lowercased) pane content.
- **FR-002**: Status MUST remain `Error` for at least 30 seconds after first detection before re-checking.
- **FR-003**: Sessions within 3 seconds of start MUST show `Starting` regardless of pane content.
- **FR-004**: A dead pane (process exited) MUST result in `Error` status.
- **FR-005**: Sessions with custom commands that return `Idle` from detection MUST show `Unknown` (custom command detection is not agent-specific).
- **FR-006**: Status detection MUST be non-blocking (pane capture has a timeout).
- **FR-007**: Each agent MUST have its own detection function registered in the `AgentDef`.
- **FR-008**: Status updates MUST clear `last_error` when the session recovers to a healthy state.

## Success Criteria

- **SC-001**: Users can distinguish Running/Waiting/Idle agents at a glance in the TUI.
- **SC-002**: Error sessions are detected within one polling interval.
- **SC-003**: Status polling does not cause visible TUI lag.
- **SC-004**: All 6 registered agents have accurate status detection.
