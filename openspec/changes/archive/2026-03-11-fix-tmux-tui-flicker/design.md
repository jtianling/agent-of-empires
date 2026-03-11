## Context

The `aoe` TUI uses `ratatui` with the `crossterm` backend. While `ratatui` performs efficient diff-based incremental updates, certain terminal environments—especially nested `tmux` sessions—can exhibit visible flickering. This happens when the terminal renders partial frame updates before a full `draw()` cycle completes, or when frequent `clear()` operations disrupt the visual continuity.

## Goals / Non-Goals

**Goals:**
- Eliminate visual flickering in `tmux` sessions during both user input and background status updates.
- Reduce terminal overhead by batching redraws and throttling background `tmux` subprocess calls.
- Maintain high input responsiveness while ensuring visual stability.

**Non-Goals:**
- Supporting legacy terminals that lack modern escape sequence support.
- Significant restructuring of the `HomeView` component hierarchy.

## Decisions

### 1. Enable Synchronized Updates
- **Decision**: Wrap the `terminal.draw()` call with `BeginSynchronizedUpdate` and `EndSynchronizedUpdate` escape sequences (via `crossterm`).
- **Rationale**: Synchronized Updates (standardized in terminals like tmux 3.4+, iTerm2, and Foot) signal the terminal to buffer all incoming changes and render them atomically as a single frame. This is the most effective way to prevent "half-drawn" frames that cause flickering.
- **Alternatives Considered**: Using a custom `BufferedBackend` was considered but rejected as `ratatui` 0.29 + `crossterm` 0.28 already provide the necessary primitives for synchronized updates.

### 2. Batch Redraw Logic in the Main Loop
- **Decision**: Refactor `App::run` to use a `refresh_needed` flag that is set by both input events and background updates, ensuring `terminal.draw()` is called exactly once at the end of each loop iteration.
- **Rationale**: The current implementation sometimes calls `draw()` immediately after a keypress and then potentially again in the same iteration if a background status update arrives. Batching ensures we only perform the expensive rendering operation once per "logical frame."
- **Implementation**: Remove immediate `draw()` calls from `handle_key` and `handle_mouse`, and move the drawing logic to a single check at the bottom of the loop.

### 3. Minimize and Audit Screen Clears
- **Decision**: Replace `terminal.clear()` with targeted background clears using the `Clear` widget only where absolutely necessary (e.g., dialog overlays).
- **Rationale**: `terminal.clear()` sends a hard reset to the terminal which is highly disruptive. In most cases, `ratatui`'s incremental rendering is sufficient to overwrite the previous frame.

## Risks / Trade-offs

- **[Risk] Unsupported Terminals** → Most modern terminals gracefully ignore synchronized update sequences if they don't support them, so there is no negative impact on older environments.
- **[Risk] Increased Input Latency** → Batching redraws at the end of a 50ms poll could theoretically add up to 50ms of latency. However, this is negligible for TUI interactions and is a worthwhile trade-off for visual stability.
