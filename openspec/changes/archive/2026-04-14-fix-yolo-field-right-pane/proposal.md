## Why

The new session dialog's "Skip permission prompts" (YOLO mode) checkbox only considers the left pane tool when deciding visibility. When the left pane is "shell" and the right pane is a code agent (e.g., claude), the checkbox is hidden entirely, creating sessions where the code agent lacks skip-permission flags.

## What Changes

- The `has_yolo` condition in the new session dialog will check both the left pane AND the right pane tool selection, showing the YOLO checkbox when either pane is a code agent that supports (and doesn't auto-enable) YOLO mode.
- A new `right_pane_needs_yolo()` helper method will be added to encapsulate the right pane's YOLO eligibility check.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `right-pane`: The right pane tool selection must influence YOLO field visibility in the new session dialog. When a right pane code agent is selected, the YOLO checkbox must appear even if the left pane is a terminal.

## Impact

- `src/tui/dialogs/new_session/mod.rs`: Add `right_pane_needs_yolo()` helper, update `has_yolo` condition in `handle_key()`
- `src/tui/dialogs/new_session/render.rs`: Update `has_yolo` condition in render logic
- No API, dependency, or data format changes
