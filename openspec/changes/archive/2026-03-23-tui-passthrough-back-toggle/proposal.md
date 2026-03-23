## Why

When a user detaches from Session A back to the AoE TUI, then selects Session B, pressing Ctrl+b b in Session B does nothing -- the previous session context was cleared on TUI entry. This forces the user to manually navigate back to Session A, breaking the "quick toggle" workflow. The TUI should act as a transparent waypoint that preserves the navigation chain, not as a context boundary that resets it.

## What Changes

- TUI remembers which session the user came from when they return to the home screen (via existing `last_detached_session` mechanism)
- `attach_to_session` sets `@aoe_prev_session_{client}` to the source session instead of unconditionally clearing it
- `attach_to_session` sets `@aoe_from_title` on the target session from the source session instead of clearing it
- If no source session exists (e.g., first TUI launch) or source equals target, falls back to current clear behavior
- Once inside a session, tmux-level navigation (Ctrl+b n/p/N/P/number/b) overwrites the TUI-seeded previous session normally

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `session-back-toggle`: "TUI entry clears stale from-title" and "TUI entry clears stale previous-session" requirements change to pass through the source session instead of clearing

## Impact

- `src/tui/app.rs`: attach_to_session flow, add field to track source session
- `src/tmux/utils.rs`: may need a helper to set (not just clear) previous session from TUI context
- `openspec/specs/session-back-toggle/spec.md`: spec update for changed requirements and scenarios
