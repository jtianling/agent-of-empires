## Why

When AoE exits, it writes an empty OSC 0 sequence (`\x1b]0;\x07`) to clear the terminal tab title. This leaves the tab with a blank title in terminals like Alacritty, iTerm2, and others. The title should be restored to whatever it was before AoE launched.

## What Changes

- On TUI startup, push the current terminal title onto the xterm title stack using CSI 22;2 t before setting AoE's title
- On normal exit, pop the saved title from the stack using CSI 23;2 t instead of clearing to empty
- On panic exit, also pop the title stack instead of clearing
- The existing `clear_terminal_title` function is replaced with a `restore_terminal_title` approach using the push/pop mechanism

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `terminal-tab-title`: Change exit behavior from "clear title to empty" to "restore original title via xterm title stack push/pop"

## Impact

- `src/tui/tab_title.rs`: Add `push_terminal_title` and `pop_terminal_title` functions using CSI 22;2 t / CSI 23;2 t sequences. `clear_terminal_title` can be removed or kept as fallback.
- `src/tui/mod.rs`: Call `push_terminal_title` before first title set on startup; call `pop_terminal_title` on both normal exit and in the panic hook instead of `clear_terminal_title`.
