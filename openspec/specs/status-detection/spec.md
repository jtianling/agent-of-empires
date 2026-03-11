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
