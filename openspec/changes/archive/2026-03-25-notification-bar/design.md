## Context

AoE manages multiple tmux sessions, each running an AI agent. The tmux status bar already shows per-session info (index, title, detach hint) via tmux user options (`@aoe_*`). A background codex title monitor pattern exists for per-session polling. The user spends most time inside agent sessions where the AoE TUI is blocked, making the tmux status bar the only surface for cross-session awareness.

## Goals / Non-Goals

**Goals:**
- Show Waiting/Idle sessions in the tmux status bar so users know when other agents need attention
- Enable quick navigation via visible index numbers matching `Ctrl+b <N>` jump keys
- Keep updates real-time (2-3 second polling) even when TUI is blocked

**Non-Goals:**
- Transition notification daemon (sending messages to parent sessions) -- future work
- Web push notifications -- out of scope
- Configurable notification modes (minimal, show-all) -- start with single default mode
- Sound/bell alerts on status transitions

## Decisions

### D1: Background daemon vs tmux `#()` shell command

**Decision**: Background daemon process (`aoe tmux monitor-notifications`).

**Rationale**: tmux `#()` output does not support style tags, so notifications would render in the same dim color as surrounding text. A daemon sets a tmux user option, which can be wrapped in tmux conditional format with distinct color. This also follows the existing codex title monitor pattern.

**Alternative considered**: `#(aoe tmux waiting-sessions --exclude '#S')` embedded in status format. Simpler (no process management) but no color control and spawns a new process every status-interval tick.

### D2: Per-session `@aoe_waiting` option with tmux conditional format

**Decision**: Store pre-formatted notification text in `@aoe_waiting` tmux user option per session. Use tmux conditional `#{?#{@aoe_waiting},...,}` in STATUS_LEFT_FORMAT.

**Rationale**: Tmux handles show/hide logic natively. No notification = empty option = nothing shown. Each session gets a different value (self excluded).

### D3: Reuse existing status detection

**Decision**: Reuse `capture-pane` based status detection from `src/tmux/status_detection.rs` and instance status from loaded session data.

**Rationale**: Status detection infrastructure is already mature. The monitor loads instances from disk (which have persisted status from the last TUI poll) and supplements with live tmux checks for sessions that may have changed.

### D4: Group collapse state from disk

**Decision**: The monitor loads `groups.json` from the profile directory to determine which groups are collapsed.

**Rationale**: Group collapse state is persisted to disk on every toggle. Reading from disk is simpler than adding tmux options for group state. The data is small and read is fast.

### D5: Single instance via tmux server option

**Decision**: Track the monitor PID in a tmux server option `@aoe_notification_monitor_pid`. On ensure, check if PID is alive. The monitor itself checks this option each cycle to detect replacement.

**Rationale**: Follows the exact pattern of `ensure_codex_title_monitor` / `run_codex_title_monitor` which uses `@aoe_codex_title_monitor_pid`. Server-level (not session-level) because the monitor serves all sessions.

### D6: Index computation

**Decision**: The monitor computes session indices using the same sort order logic as `update_session_index()` in `src/tmux/utils.rs`, loading instances + sort preferences.

**Rationale**: Index numbers must match what `Ctrl+b <N>` resolves to, which uses the same sort order. Reusing the same logic ensures consistency.

## Risks / Trade-offs

- **[Stale group state]** Group collapse state is read from disk; if the user toggles collapse in TUI, the monitor picks it up on the next cycle (2-3s delay). Acceptable for this use case. -> Mitigation: short poll interval.
- **[Performance with many sessions]** The monitor does `capture-pane` for status detection on each session. With 20+ sessions this adds subprocess overhead. -> Mitigation: for now, the monitor can skip full status detection and rely on the persisted status from `sessions.json` (written by TUI's StatusPoller). Live `capture-pane` only needed when TUI is not running. Actually, using persisted status is sufficient since the TUI updates it before attaching.
- **[Monitor orphan]** If AoE crashes without cleanup, the monitor continues running. -> Mitigation: monitor self-exits when no `aoe_*` sessions remain. Also checks PID ownership each cycle.
- **[Status-left truncation]** Even with 160 chars, many sessions could overflow. -> Mitigation: acceptable for v1; a future `max_shown` config can limit displayed count.
