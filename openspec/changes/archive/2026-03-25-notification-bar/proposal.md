## Why

When a user is working inside an agent session, they have no visibility into other sessions' states. If another agent finishes and enters Waiting or Idle, the user won't know until they manually switch back to the AoE TUI. This creates unnecessary context-switching overhead, especially when managing multiple concurrent agents.

## What Changes

- Add a notification bar to the tmux status bar that shows Waiting and Idle sessions alongside the existing "Ctrl+b d detach" hint.
- Display format: `| [index] title [index] title` appended after "Ctrl+b d detach", separated by ` | `.
- A background notification monitor daemon (similar to the existing codex title monitor) polls session statuses every 2-3 seconds and updates a per-session tmux user option (`@aoe_waiting`).
- Waiting sessions are always shown. Idle sessions are shown only if they are NOT inside a collapsed group.
- The notification text is colored distinctly (yellow/colour220) to stand out from the dim hint text.
- Each session in the notification includes its index number (matching the `Ctrl+b <N>` jump key), enabling quick navigation.

## Capabilities

### New Capabilities
- `notification-bar`: Tmux status bar notification showing Waiting/Idle sessions with index-based quick-jump hints. Includes a background monitor daemon for real-time updates.

### Modified Capabilities

## Impact

- `src/tmux/status_bar.rs`: Modified STATUS_LEFT_FORMAT, new monitor daemon functions, increased status-left-length.
- `src/cli/tmux.rs`: New `monitor-notifications` subcommand.
- `src/session/instance.rs`: Trigger notification monitor on session creation.
- Depends on existing status detection (`capture-pane` based), group tree loading, and tmux user option infrastructure.
