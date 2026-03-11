## Why

When aoe is running in the background (e.g., behind other windows or in another tmux pane/terminal tab), users have no way to know when something needs their attention -- like a session finishing creation or a dialog requiring input. Gemini CLI solves this by updating the terminal tab title with state-specific icons, making it easy to glance at a tab bar and know what's happening. This is especially useful in terminals like Alacritty, iTerm2, and kitty that display tab titles prominently.

## What Changes

- Add a terminal tab title module that writes OSC escape sequences (`\x1b]0;...\x07`) to set the terminal window/tab title
- Update the title dynamically based on TUI state: idle/home view, dialog requiring input, settings open, diff view, session creating, etc.
- Use distinctive emoji/icons for each state so users can identify status at a glance from the tab bar
- Restore the original terminal title on exit (reset with empty OSC sequence)
- Add a configuration option to disable the feature (`ui.dynamic_tab_title`, default enabled)

## Capabilities

### New Capabilities
- `terminal-tab-title`: Dynamic terminal tab/window title that reflects the current TUI state using OSC escape sequences and state-specific icons

### Modified Capabilities
- `configuration`: Add `dynamic_tab_title` boolean field to UI configuration
- `tui`: Integrate title updates into the TUI event loop and terminal setup/teardown

## Impact

- **Code**: New module `src/tui/tab_title.rs`; changes to `src/tui/app.rs` (event loop), `src/tui/mod.rs` (setup/teardown), and configuration structs
- **Dependencies**: No new crate dependencies -- uses raw ANSI/OSC escape sequences via `crossterm::execute!` or direct stdout writes
- **Systems**: Only affects terminals that support OSC title sequences (most modern terminals). Terminals that don't support it will silently ignore the escape codes.
