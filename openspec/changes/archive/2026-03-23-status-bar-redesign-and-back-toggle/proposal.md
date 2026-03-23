## Why

With Ctrl+b 1-9 number jumping and Ctrl+b N/P global cycling now in place, users frequently jump between sessions but lose track of which numbered session they're in and which session they came from. The current status bar wastes space on the raw tmux session name (#S), a redundant title in status-right, and a noisy window list -- leaving no room for the information that actually matters during rapid session switching.

## What Changes

- Redesign the tmux status bar layout:
  - status-left: show session index number, session title, conditional "from: <title>" when a back target exists, and a single hint (Ctrl+b d detach)
  - status-right: remove "aoe: Title" prefix, keep only branch/sandbox/time
  - Hide the tmux window list (window-status-format) to eliminate the "0:name*" noise in the middle
- Add Ctrl+b b keybinding to toggle back to the previous session
  - All jump paths (n/p group cycle, N/P global cycle, 1-9 number jump, b back) record the source session as the "previous session"
  - Ctrl+b b reads the previous session and switches to it, creating a toggle between two sessions
- Set @aoe_index on the target session each time a switch occurs, so the status bar always shows the current session's number
- Set @aoe_from_title on the target session each time a switch occurs, so the status bar conditionally shows where the user came from

## Capabilities

### New Capabilities
- `session-back-toggle`: Ctrl+b b keybinding that toggles back to the previous session, with previous-session tracking across all jump types

### Modified Capabilities
- `number-jump`: Add @aoe_index tracking on session entry (index is computed and stored as a tmux option so the status bar can display it)

## Impact

- `src/tmux/status_bar.rs`: Complete rewrite of status-left/status-right format strings, add window-list hiding, new @aoe_index and @aoe_from_title options
- `src/tmux/utils.rs`: Add previous-session tracking to switch_aoe_session and switch_aoe_session_by_index, new switch_aoe_session_back function, bind/unbind/cleanup for Ctrl+b b in both nested and non-nested modes
- `src/cli/tmux.rs`: Add --back flag to SwitchSessionArgs
- Tests: Update status_bar format tests, add unit tests for back-toggle logic
