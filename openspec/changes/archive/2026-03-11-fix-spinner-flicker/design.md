## Context

While previous fixes introduced Synchronized Output and batching, the current implementation still exhibits flickering during spinner animations. This is primarily due to state changes (like preview cache refreshes) occurring *during* the `terminal.draw` call, which then signals that *another* redraw is needed in the very next loop iteration. This "feedback loop" of redraws, combined with high-frequency animation ticks, causes visible flickering.

## Goals / Non-Goals

**Goals:**
- Eliminate flickering during "thinking" spinners and hook execution.
- Ensure the TUI state is "settled" before any drawing begins.
- Reduce the frequency of redundant `terminal.draw` calls.

**Non-Goals:**
- Removing the `Clear` widget (it is necessary for correct dialog rendering).
- Changing the visual style of the spinners.

## Decisions

### 1. Pre-fetch and Pre-settle State
- **Decision**: Move the preview cache refresh logic out of `HomeView::render` and into a dedicated `HomeView::update` or similar pre-draw phase.
- **Rationale**: Rendering should be a "pure" function of the current state. By the time `draw()` is called, all data (including tmux captures) should already be in memory. This prevents the "render triggers redraw" cycle.

### 2. Global Redraw Throttling for Animations
- **Decision**: In `App::run`, track the last time a redraw was triggered purely by a "tick" (spinner) event. Limit these to a maximum frequency (e.g., 10Hz / 100ms).
- **Rationale**: User input (keypresses) should still feel instant, but visual-only animations can be throttled without degrading perceived quality, while significantly improving stability in tmux.

### 3. Refactor `App::run` Iteration
- **Decision**: Re-order the loop: 
    1. Check for inputs.
    2. Check for background status updates.
    3. Update caches (move logic from render to here).
    4. Handle logic ticks.
    5. Determine if redraw is needed.
    6. DRAW.
- **Rationale**: Ensures that when `terminal.draw` is finally called, no further state changes will be pending for that logical frame.

## Risks / Trade-offs

- **[Risk] Sync Issues** → If cache updates are missed before drawing, the UI might be one frame behind.
- **[Mitigation]** → Ensure `refresh_needed` is set correctly by the new pre-draw update methods.
