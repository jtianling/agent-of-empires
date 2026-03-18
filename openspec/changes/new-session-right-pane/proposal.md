## Why

Creating a dual-pane layout in AoE requires multiple manual steps: creating a session, entering it, running `Ctrl+b %` to split, then launching a tool in the new pane. This is tedious for users who frequently work with side-by-side code agents. Adding a "Right Pane" option to the new session dialog eliminates this friction.

## What Changes

- Add a "Right Pane" tool selector field to the new session dialog, placed below the existing Tool field
- The right pane field offers the same tool choices as the main Tool field but defaults to "none" (unselected)
- When "none" is selected, session creation behaves identically to today
- When a tool is selected, AoE automatically splits the tmux session horizontally (`split-window -h`) after creating the main session, and launches the chosen tool in the right pane
- The right pane tool selection is stored in `NewSessionData` and passed through the session creation flow
- The `@aoe_agent_pane` tracking (which stores the left/agent pane ID) must remain correct so that status detection, `remain-on-exit`, and detach/reattach all target the left pane properly -- avoiding the same class of bug fixed in commit `ab54364`

## Capabilities

### New Capabilities
- `right-pane`: Automatic right pane creation with tool selection in the new session dialog

### Modified Capabilities
- `session-management`: Session creation flow gains optional right pane splitting after tmux session creation

## Impact

- **Code**: `src/tui/dialogs/new_session/` (dialog fields, rendering, data), `src/session/instance.rs` (post-creation split), `src/tui/home/operations.rs` (pass right pane data), `src/tmux/session.rs` or `src/tmux/utils.rs` (split-window helper)
- **Data**: `NewSessionData` struct gains `right_pane_tool` field; `Instance` may optionally store right pane info for restart scenarios
- **Risk**: Pane ID tracking (`@aoe_agent_pane`) must remain correct after the split; the right pane must not interfere with status detection, remain-on-exit, or detach bindings
