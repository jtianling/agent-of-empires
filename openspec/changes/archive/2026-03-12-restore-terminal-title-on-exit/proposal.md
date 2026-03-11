## Why

When AoE exits, it clears the terminal tab title by writing an empty OSC 0 sequence, leaving the Alacritty (and other terminal emulator) tab title blank. The title should be restored to whatever it was before AoE launched, not cleared to empty.

## What Changes

- Save the original terminal title on TUI startup before setting AoE's title
- On exit (normal and panic), restore the saved original title instead of clearing to empty
- Use OSC 21 (XTGETTITLE) to query the current terminal title, with a fallback for terminals that don't support it

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `terminal-tab-title`: Change exit behavior from "clear title" to "restore original title"

## Impact

- `src/tui/tab_title.rs`: Add save/restore functions using xterm title query (OSC 21)
- `src/tui/mod.rs`: Save title before setting, restore on exit instead of clearing
- `src/tui/app.rs`: Pass saved title context if needed
