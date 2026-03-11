## Why

Users experience significant TUI flickering when running `aoe` inside a `tmux` session, especially during agent output streaming in the preview window or during rapid user input. This degrades the user experience and makes the tool feel less stable compared to other CLIs like `claude code`.

## What Changes

- Implement `Synchronized Output` support in the TUI backend to batch terminal updates into single frames, preventing partial redraws from being visible.
- Optimize the TUI main loop to ensure `terminal.draw()` is called efficiently and only once per frame when multiple updates occur simultaneously.
- Tune the refresh frequency of the session preview to balance responsiveness with the overhead of `tmux capture-pane` calls.
- Audit and minimize the use of `terminal.clear()` to avoid disruptive full-screen blinks.

## Capabilities

### New Capabilities
- None

### Modified Capabilities
- `tui`: Update TUI requirements to include flickering prevention and improved compatibility for nested tmux environments.

## Impact

- `src/tui/mod.rs`: Initialization and cleanup of synchronized output features.
- `src/tui/app.rs`: Refinement of the main execution loop and redraw logic.
- `src/tui/home/mod.rs`: Configuration and timing of background preview refreshes.
