## Why

When Gemini is in a "thinking" state, the spinner animation (⠼) causes intense flickering in the input box and surrounding UI elements when running inside a `tmux` session. This happens because high-frequency animation ticks combined with Ratatui's `Clear` widget and nested terminal latency create a visible "wipe-and-fill" effect that Synchronized Output alone hasn't fully eliminated.

## What Changes

- **Decouple Animation Ticks**: Limit the frequency of UI redraws triggered by spinner animations to a stable rate (e.g., 100-150ms) rather than every 50ms loop iteration.
- **Pre-Render State Finalization**: Move all cache refreshes and terminal status updates *before* the `terminal.draw` call, ensuring the TUI doesn't request a "second frame" immediately after the first one due to state changes discovered during rendering.
- **Optimized Dialog Overlay**: Refine how dialogs (which contain the spinners) interact with the background to minimize the area being cleared and redrawn.
- **Atomic Event Handling**: Ensure that a single loop iteration handles all pending events before deciding whether a redraw is needed.

## Capabilities

### New Capabilities
- None

### Modified Capabilities
- `tui`: Refine rendering stability requirements to specifically address high-frequency UI animations (spinners).

## Impact

- `src/tui/app.rs`: Refactor the loop to ensure state is fully settled before drawing.
- `src/tui/home/mod.rs` & `src/tui/home/render.rs`: Move cache logic out of the rendering path.
- `src/tui/dialogs/*.rs`: Review spinner animation timing.
