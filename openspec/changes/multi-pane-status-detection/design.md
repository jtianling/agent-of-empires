## Context

AoE creates one tmux session per Instance, launching an agent command in pane 0. The pane ID is stored in `@aoe_agent_pane`, and all status detection targets only this pane. Users frequently split sessions into multiple panes (Ctrl+b %, Ctrl+b ") to run additional agents. These extra panes are invisible to AoE.

Current pane info cache (`src/tmux/mod.rs`) stores `HashMap<session_name, PaneInfo>`, keeping only the lowest-indexed pane per session. Status detection in `update_status_with_options()` reads hook files or captures content from the single agent pane.

Key observation from live system: `pane_current_command` shows `2.1.81` (version) for Claude Code instead of "claude", but `ps -o comm -p <pane_pid>` correctly shows "claude" for AoE-created panes. For user-split panes, `pane_pid` is a shell (zsh), and the foreground child process is the agent.

## Goals / Non-Goals

**Goals:**
- Detect and aggregate status across all non-shell panes in a session
- Automatically identify which agent type is running in each pane
- Use priority aggregation: Waiting > Running > Idle
- Implement content-based Claude Code status detection (currently a hook-only stub)
- Work in both TUI status poller and notification monitor

**Non-Goals:**
- Managing or creating extra panes (users do this manually)
- Hook injection into user-started agents (hooks only work for AoE-launched instances)
- Per-pane status display in the TUI (aggregated session-level status only)
- Monitoring panes in non-AoE tmux sessions

## Decisions

### D1: Pane info cache stores all panes per session

Change `HashMap<String, PaneInfo>` to `HashMap<String, Vec<PaneInfo>>`. The existing `tmux list-panes -a` call already returns all panes -- we just stopped discarding extras. Add `pane_index: u32` and `pane_id: String` to `PaneInfo`.

The existing single-pane accessor `get_cached_pane_info(session_name)` returns the agent pane (pane 0 or `@aoe_agent_pane` match). A new `get_all_cached_pane_infos(session_name)` returns all panes.

**Alternative**: Separate cache for extra panes. Rejected -- adds complexity with no benefit since we already parse all panes from `list-panes -a`.

### D2: Agent type detection via process inspection

Detection chain per pane:

1. **`pane_current_command` quick match**: Check against known agent binary names and shell names. Handles Codex (`codex-*`), OpenCode, Gemini, Vibe directly. Shell names (bash, zsh, fish, sh, dash, ksh, tcsh) classify as shell.
2. **`pane_pid` comm name**: For ambiguous `pane_current_command` (like Claude's `2.1.81`), get the process comm via `ps -o comm= -p <pane_pid>`. If pane_pid itself is the agent (AoE-created panes), match directly.
3. **Foreground process from shell**: For user-split panes where pane_pid is a shell, use `get_foreground_pid(pane_pid)` then check comm name of the result. The foreground PID detection already exists in `src/process/{macos,linux}.rs`.
4. **Fallback**: If nothing matches, classify as shell (ignored for aggregation).

Detection order matches user's preference: Claude Code -> Codex -> Gemini -> OpenCode -> Shell.

**Alternative**: Content-based agent type detection (inspect pane output patterns). Rejected as primary method -- too fragile and requires capture-pane for every pane. Used only as last resort if process detection fails.

### D3: Per-pane status detection reuses existing agent detection functions

For each non-shell pane, run the same detection hierarchy as the current single-pane path:
1. Title spinner detection (universal, uses cached pane title)
2. Content-based detection using the detected agent's `detect_status` function

Hook-based detection only applies to the AoE-created agent pane (pane 0) since user-started agents don't write hook files. This is acceptable because:
- Claude Code: We implement content-based detection (D4) as fallback
- Cursor: Also hook-based, same fallback needed if ever used in split panes
- Other agents: Already have full content detection

### D4: Implement Claude Code content-based detection

The current `detect_claude_status()` is a stub returning `Idle` (relies entirely on hooks). For user-split panes without hooks, we need real content detection.

Claude Code terminal output patterns:
- **Running**: Braille spinner characters in output (same as title detection), streaming text with tool-use indicators
- **Waiting**: Permission prompts (`Allow`, `Deny`), tool approval dialogs, "yes/no" prompts, MCP tool approval
- **Idle**: Input prompt `>` at bottom of screen, "What would you like to do?" type prompts

This is the same approach used by Codex, OpenCode, and Gemini detection functions.

### D5: Status aggregation logic

```
fn aggregate_pane_statuses(statuses: &[Status]) -> Status {
    // Priority: Waiting > Running > Idle
    // Error/Unknown/Starting are treated per their original meaning
    if statuses.iter().any(|s| *s == Status::Waiting) { return Status::Waiting; }
    if statuses.iter().any(|s| *s == Status::Running) { return Status::Running; }
    Status::Idle
}
```

The aggregated status replaces the single-pane status in all consumers (TUI, notification bar).

Spike detection and spinner grace period apply per-pane before aggregation, not after. The acknowledged-waiting mapping applies to the aggregated result (if the session is acknowledged, the Waiting from any pane becomes Idle).

### D6: Performance budget

Current polling: 1 pane capture per session per poll cycle (1-3s interval).

With multi-pane: N pane captures per session (where N = non-shell panes). For a typical session with 2-3 agents, this means 2-3x more capture-pane calls.

Mitigations:
- Title spinner detection (layer 1) is free -- uses cached pane title, no capture needed
- Activity gating still works at session level -- if no activity in the session, skip all pane captures
- Most users have 1-3 extra panes, not 10

Acceptable trade-off: 2-3x more tmux calls in exchange for complete status visibility.

## Risks / Trade-offs

- **[Risk] Claude content detection accuracy**: Claude Code's UI changes between versions. Patterns may break on updates. -> Mitigation: Use broad patterns (permission keywords, spinner chars), version-independent when possible. Title spinner detection remains the primary fast path.
- **[Risk] Process detection adds latency**: `ps` calls for agent type detection add overhead. -> Mitigation: Agent type detection results cached alongside pane info (2s TTL). Only re-detect when pane list changes.
- **[Risk] Foreground PID detection is OS-specific**: macOS and Linux have different APIs. -> Mitigation: Both already implemented in `src/process/`. Detection function dispatches to correct platform.
- **[Trade-off] User-split Claude panes miss Waiting sometimes**: Without hooks, content detection may not catch all Waiting states. -> Acceptable: Title spinner catches Running reliably; content detection catches most Waiting states; fallback is showing Idle (safe default).
