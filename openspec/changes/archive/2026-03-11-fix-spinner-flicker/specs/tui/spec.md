## ADDED Requirements

### Requirement: Throttled Animation Redraws
The TUI MUST throttle the frequency of redraws triggered by purely visual animations (like spinners) to prevent visual artifacts in high-latency terminal environments.

#### Scenario: Spinner Throttling
- **WHEN** a UI component (like a dialog) is animating a spinner
- **THEN** it SHALL NOT trigger a TUI-wide redraw more frequently than every 100ms, regardless of the internal tick frequency of the animation.

### Requirement: Pre-Draw State Settlement
The TUI main loop MUST ensure that all internal state updates, cache refreshes, and terminal status checks are completed *before* initiating a draw operation.

#### Scenario: Settled Drawing
- **WHEN** the TUI main loop processes an iteration
- **THEN** it SHALL perform all data fetching (like `tmux capture-pane`) and state mutations first, and only then perform at most one `terminal.draw()` call if changes occurred.
