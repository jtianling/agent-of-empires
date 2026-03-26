## Why

AoE currently only monitors the single "agent pane" (pane 0) it creates per tmux session. Users frequently split panes (Ctrl+b %, Ctrl+b ") to run additional agents, but these extra panes are completely invisible to AoE's status detection, notification bar, and TUI. A session could have 3 agents all Waiting for input, but AoE only knows about one.

## What Changes

- Expand the pane info cache from single-pane-per-session to all-panes-per-session
- Add agent type detection for arbitrary panes (detect which agent binary is running via process inspection)
- Detect status for each non-shell pane independently using the appropriate agent's detection function
- Aggregate multi-pane statuses with priority: Waiting > Running > Idle
- Both TUI status poller and notification monitor use the aggregated status
- Implement content-based status detection for Claude Code (currently a stub relying on hooks, which don't exist for user-created panes)

## Capabilities

### New Capabilities
- `multi-pane-status`: Detect agent type and aggregate status across all panes in a tmux session

### Modified Capabilities
- `status-detection`: Add content-based detection for Claude Code; extend detection pipeline to support per-pane detection with different agent types

## Impact

- `src/tmux/mod.rs`: Pane info cache restructured to store all panes per session
- `src/tmux/status_detection.rs`: New Claude Code content detection; new agent type detection from process info
- `src/session/instance.rs`: `update_status_with_options()` extended for multi-pane aggregation
- `src/tui/status_poller.rs`: Uses aggregated multi-pane status
- `src/tmux/notification_monitor.rs`: Uses aggregated multi-pane status
- `src/process/macos.rs` and `src/process/linux.rs`: May need utility for getting process comm name from PID
