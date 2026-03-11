## ADDED Requirements

### Requirement: Flickering Prevention in tmux
The TUI MUST implement rendering optimizations to prevent visible flickering when running inside a `tmux` session. This ensures a smooth user experience even during rapid state updates or high-frequency input.

#### Scenario: Synchronized Output
- **WHEN** the TUI renders a frame in a terminal that supports Synchronized Output (e.g., modern tmux 3.4+ or iTerm2)
- **THEN** it SHALL use the terminal's synchronized update sequences to ensure the frame is displayed atomically, preventing partial redraws from being visible.

#### Scenario: Redraw Batching
- **WHEN** multiple state changes occur within a single TUI loop iteration (e.g., a background status update completes at the same time as a keystroke event is processed)
- **THEN** the TUI SHALL batch these changes and perform at most one `terminal.draw()` call for that frame to avoid redundant redraw operations.

#### Scenario: Minimal Screen Clears
- **WHEN** the TUI is running in its normal event loop
- **THEN** it SHALL NOT call `terminal.clear()` unless the terminal state is explicitly known to be corrupted, after returning from an external full-screen process (like attaching to a tmux session), or when switching between major TUI views that require a full reset.

### Requirement: Optimized Preview Refresh
The TUI SHALL optimize the session preview refresh rate and rendering to reduce the performance impact of background `tmux capture-pane` calls.

#### Scenario: Throttled Preview
- **WHEN** a session is selected and its output is being previewed in the TUI
- **THEN** the TUI SHALL throttle background refreshes of the preview content to a stable rate (e.g., 250ms interval) to minimize the frequency of `tmux` subprocess calls while maintaining acceptable live feedback.
