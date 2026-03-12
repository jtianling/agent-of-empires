## Context

AoE currently relies on two different title paths:

- The AoE TUI owns the outer terminal title while the dashboard is active.
- Attached tmux sessions propagate the active pane title to the outer terminal title through
  session-scoped tmux options.

That split works for agents that emit their own OSC title updates, but Codex CLI does not currently
write terminal titles itself. The current local fix attempt only changes the TUI background status
poller, which means the raised-hand waiting title disappears in the live attached Codex session and
does not satisfy the actual user workflow.

## Goals / Non-Goals

**Goals:**

- Make the Codex CLI session title show `✋ <session title>` whenever Codex is waiting for user
  input or approval.
- Keep the plain session title for Codex when it is running or idle.
- Make the behavior work in the real tmux session lifecycle, including attach flows.
- Avoid changing title ownership or waiting-title behavior for other agents.

**Non-Goals:**

- Reworking title handling for Claude, Gemini, OpenCode, Vibe, Cursor, or paired terminal sessions.
- Reintroducing dynamic AoE TUI view-based titles.
- Changing Codex status-detection semantics beyond reusing the existing waiting/running/idle logic.

## Decisions

### 1. Use a Codex-only session-local title monitor

AoE will start or refresh a small Codex-only monitor for Codex agent sessions. The monitor will poll
the tmux pane content for that session, derive the current Codex status with the existing detection
logic, and update the pane title with `tmux select-pane -T`.

This keeps the behavior tied to the actual Codex tmux session instead of the AoE dashboard process,
so it continues to work while the user is attached to Codex.

Alternative considered: extend `src/tui/status_poller.rs` only. Rejected because the dashboard
poller is not the active control path while the user is attached to the Codex session.

### 2. Reuse the existing Codex waiting detection rules

The monitor will reuse `detect_codex_status` so Codex title updates and AoE status reporting share
the same interpretation of Codex UI states. Only `Status::Waiting` maps to the raised-hand title;
all other states map back to the plain session title.

Alternative considered: add separate title-specific parsing rules. Rejected because it would create
two drifting definitions of "Codex is waiting."

### 3. Keep title ownership unchanged for non-Codex sessions

The Codex waiting-title fix will be isolated to Codex session startup and attach refresh paths.
AoE will not broaden or narrow title ownership for other agents, and it will not convert the generic
managed-pane-title path into Codex behavior.

Alternative considered: change the shared `sets_own_title = false` path or make all waiting sessions
use the raised-hand icon. Rejected because the user explicitly requested a Codex-only change.

### 4. Ensure existing Codex sessions can pick up the behavior on attach

Attach paths will ensure Codex sessions have the title monitor running even if the session was
created before this change or before the monitor was started. This keeps the fix effective without
requiring the user to recreate every existing Codex session.

Alternative considered: require restart or recreation. Rejected because the repository guidance for
title behavior explicitly calls out refreshing existing sessions through attach helpers when needed.

## Risks / Trade-offs

- [Risk] A polling monitor could add unnecessary tmux traffic. -> Mitigation: deduplicate pane-title
  writes and use a modest polling interval.
- [Risk] Duplicate monitors could race and flicker titles. -> Mitigation: track monitor ownership per
  tmux session and reuse or restart only when needed.
- [Risk] Codex detection patterns may evolve upstream. -> Mitigation: reuse the existing centralized
  Codex status detection function and cover the title path with focused tests.

## Migration Plan

- No data migration is required.
- Existing Codex sessions should pick up the monitor through the session attach refresh path.
- Restarting a Codex session remains a valid fallback if a monitor was not previously running.

## Open Questions

- None.
